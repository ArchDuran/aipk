use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::format::{
    header_flags, parse_bytes, AipkPackage, HEADER_SIZE, PKG_FLAG_ENCRYPTED, PKG_FLAG_SEALED,
    SECTION_HEADER_SIZE,
};

// ─── constants ───────────────────────────────────────────────────────────────

pub const SECTION_FLAG_ENCRYPTED: u32 = 0x08;
const NONCE_SIZE: usize = 12;

/// Sections that are encrypted by default (content). META, INDX, SIGN are always plain.
/// ANNX must travel with KNOW: it embeds a normalized copy of the same
/// vectors for HNSW search, so leaving it plaintext while KNOW is encrypted
/// would defeat the point — the "index" section would just leak the content.
const DEFAULT_ENCRYPT_SECTIONS: &[&str] = &[
    "PERS", "KNOW", "ANNX", "SKIL", "CLMS", "CLMV", "SRCS", "IDTY", "ANSP", "PLCY", "NKNW", "TOOL",
    "THKG", "TEST",
];

// ─── Key derivation ───────────────────────────────────────────────────────────
//
// Passphrase-based encryption (`aipk encrypt`/`decrypt`) derives its AES key with
// Argon2id, not a bare hash: a human passphrase has far less entropy than a random
// 256-bit key, and a bare SHA-256 lets an attacker with the ciphertext try billions
// of guesses per second on a GPU. Argon2id makes each guess cost real memory and
// time. The salt (random per encryption) and the params travel with the package
// in a plaintext `PKDF` section, so a fixed passphrase never reuses a key across
// packages and future defaults can change without breaking old ones (`version`).

const PKDF_VERSION: u8 = 1;
const PKDF_ALGO_ARGON2ID: u8 = 1;
const PKDF_SALT_LEN: usize = 16;
const PKDF_DATA_LEN: usize = 1 + 1 + 4 + 4 + 4 + PKDF_SALT_LEN; // 30 bytes

/// OWASP-recommended Argon2id minimum: 19 MiB memory, 2 iterations, 1 lane.
const DEFAULT_M_COST_KIB: u32 = 19 * 1024;
const DEFAULT_T_COST: u32 = 2;
const DEFAULT_P_COST: u32 = 1;

#[derive(Clone, Copy)]
struct KdfParams {
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: DEFAULT_M_COST_KIB,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
        }
    }
}

fn derive_key_argon2(
    passphrase: &str,
    salt: &[u8; PKDF_SALT_LEN],
    params: KdfParams,
) -> Result<[u8; 32]> {
    let argon2_params = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|e| anyhow!("invalid Argon2 parameters: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("Argon2 key derivation failed: {e}"))?;
    Ok(key)
}

fn random_salt() -> [u8; PKDF_SALT_LEN] {
    let mut salt = [0u8; PKDF_SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

fn build_pkdf_section(salt: &[u8; PKDF_SALT_LEN], params: KdfParams) -> Vec<u8> {
    let mut data = Vec::with_capacity(PKDF_DATA_LEN);
    data.push(PKDF_VERSION);
    data.push(PKDF_ALGO_ARGON2ID);
    data.extend_from_slice(&params.m_cost_kib.to_le_bytes());
    data.extend_from_slice(&params.t_cost.to_le_bytes());
    data.extend_from_slice(&params.p_cost.to_le_bytes());
    data.extend_from_slice(salt);
    data
}

fn parse_pkdf_section(data: &[u8]) -> Result<([u8; PKDF_SALT_LEN], KdfParams)> {
    if data.len() < PKDF_DATA_LEN {
        anyhow::bail!(
            "PKDF section too short ({} bytes, need {PKDF_DATA_LEN})",
            data.len()
        );
    }
    if data[0] != PKDF_VERSION {
        anyhow::bail!("unsupported PKDF section version {}", data[0]);
    }
    if data[1] != PKDF_ALGO_ARGON2ID {
        anyhow::bail!("unsupported KDF algorithm id {}", data[1]);
    }
    let m_cost_kib = u32::from_le_bytes(data[2..6].try_into().unwrap());
    let t_cost = u32::from_le_bytes(data[6..10].try_into().unwrap());
    let p_cost = u32::from_le_bytes(data[10..14].try_into().unwrap());
    let salt: [u8; PKDF_SALT_LEN] = data[14..14 + PKDF_SALT_LEN].try_into().unwrap();
    Ok((
        salt,
        KdfParams {
            m_cost_kib,
            t_cost,
            p_cost,
        },
    ))
}

/// Encrypt a package with a passphrase: derives an Argon2id key behind a fresh
/// random salt and embeds it (with the KDF params) in a plaintext `PKDF` section
/// so `decrypt_pkg_with_passphrase` can reproduce the same key later.
pub fn encrypt_pkg_with_passphrase(
    raw: &[u8],
    passphrase: &str,
    section_filter: &[String],
) -> Result<Vec<u8>> {
    let salt = random_salt();
    let params = KdfParams::default();
    let key = derive_key_argon2(passphrase, &salt, params)?;
    let encrypted = encrypt_pkg_raw(raw, &key, section_filter)?;
    rebuild_raw(
        &encrypted,
        &[],
        &[("PKDF", build_pkdf_section(&salt, params), 0)],
        0,
        0,
    )
}

/// Decrypt a passphrase-encrypted package: reads the salt/params from its `PKDF`
/// section, re-derives the Argon2id key, decrypts, and drops the now-unneeded
/// `PKDF` section from the result.
pub fn decrypt_pkg_with_passphrase(raw: Vec<u8>, passphrase: &str) -> Result<Vec<u8>> {
    let pkg = parse_bytes(raw.clone())?;
    let pkdf_sec = pkg.section("PKDF").ok_or_else(|| {
        anyhow!("package is encrypted but has no PKDF section — cannot derive the decryption key")
    })?;
    let (salt, params) = parse_pkdf_section(pkg.section_data(pkdf_sec))?;
    let key = derive_key_argon2(passphrase, &salt, params)?;
    let decrypted = decrypt_pkg_raw(raw, &key)?;
    rebuild_raw(&decrypted, &["PKDF"], &[], 0, 0)
}

// ─── Low-level encrypt / decrypt ─────────────────────────────────────────────

/// Encrypt `plaintext` and return `[nonce 12B][ciphertext + GCM tag 16B]`.
/// AAD = section tag bytes, binds ciphertext to its section type.
pub fn encrypt_section(key: &[u8; 32], tag: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let aad = tag_aad(tag);

    let ciphertext = cipher
        .encrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("AES-GCM encryption failed for section {tag}"))?;

    let mut out = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt `[nonce 12B][ciphertext + GCM tag 16B]` and return plaintext.
pub fn decrypt_section(key: &[u8; 32], tag: &str, encrypted: &[u8]) -> Result<Vec<u8>> {
    if encrypted.len() < NONCE_SIZE + 16 {
        anyhow::bail!("encrypted data for section {tag} is too short");
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&encrypted[..NONCE_SIZE]);
    let ciphertext = &encrypted[NONCE_SIZE..];
    let aad = tag_aad(tag);

    cipher
        .decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| {
            anyhow!("AES-GCM authentication failed for section {tag} — wrong key or tampered data")
        })
}

fn tag_aad(tag: &str) -> [u8; 4] {
    let mut aad = [0u8; 4];
    for (i, b) in tag.bytes().take(4).enumerate() {
        aad[i] = b;
    }
    aad
}

// ─── Package-level encrypt / decrypt ─────────────────────────────────────────

/// Encrypt selected sections of a package and return new raw bytes.
/// `section_filter`: if empty, encrypts DEFAULT_ENCRYPT_SECTIONS.
pub fn encrypt_pkg_raw(raw: &[u8], key: &[u8; 32], section_filter: &[String]) -> Result<Vec<u8>> {
    let pkg = parse_bytes(raw.to_vec())?;
    let to_encrypt: Vec<&str> = if section_filter.is_empty() {
        DEFAULT_ENCRYPT_SECTIONS.to_vec()
    } else {
        section_filter.iter().map(|s| s.as_str()).collect()
    };
    rebuild_pkg_raw(&pkg, key, &to_encrypt, true)
}

/// Decrypt all encrypted sections and return new raw bytes with flags cleared.
pub fn decrypt_pkg_raw(raw: Vec<u8>, key: &[u8; 32]) -> Result<Vec<u8>> {
    let pkg = parse_bytes(raw)?;
    rebuild_pkg_raw(&pkg, key, &[], false)
}

/// Check if a package has any encrypted sections.
pub fn is_encrypted(raw: &[u8]) -> bool {
    let Ok(pkg) = parse_bytes(raw.to_vec()) else {
        return false;
    };
    pkg.sections
        .iter()
        .any(|s| s.flags & SECTION_FLAG_ENCRYPTED != 0)
}

// ─── internal: rebuild raw bytes ─────────────────────────────────────────────

fn rebuild_pkg_raw(
    pkg: &AipkPackage,
    key: &[u8; 32],
    encrypt_tags: &[&str],
    encrypting: bool,
) -> Result<Vec<u8>> {
    // Preserve original header (name, timestamp, etc.) except section_count and indx_offset.
    // We'll patch those at the end.
    let mut header = pkg.raw[..HEADER_SIZE].to_vec();

    let mut sections_bytes: Vec<u8> = Vec::new();
    let mut indx_entries: Vec<(String, usize, usize)> = Vec::new();
    let mut pos = HEADER_SIZE;

    for sec in &pkg.sections {
        if sec.tag == "INDX" {
            continue; // rebuilt at the end
        }

        let data = &pkg.raw[sec.offset..sec.offset + sec.size];
        let (new_data, new_flags) = if encrypting && encrypt_tags.contains(&sec.tag.as_str()) {
            // Encrypt this section
            let enc = encrypt_section(key, &sec.tag, data)?;
            (enc, sec.flags | SECTION_FLAG_ENCRYPTED)
        } else if !encrypting && sec.flags & SECTION_FLAG_ENCRYPTED != 0 {
            // Decrypt this section
            let plain = decrypt_section(key, &sec.tag, data)?;
            (plain, sec.flags & !SECTION_FLAG_ENCRYPTED)
        } else {
            (data.to_vec(), sec.flags)
        };

        let data_offset = pos + SECTION_HEADER_SIZE;
        indx_entries.push((sec.tag.clone(), data_offset, new_data.len()));
        sections_bytes.extend(write_section_with_flags(&sec.tag, &new_data, new_flags));
        pos += SECTION_HEADER_SIZE + new_data.len();
    }

    // Build INDX
    let indx_offset = pos;
    let mut indx_data = (indx_entries.len() as u32).to_le_bytes().to_vec();
    for (tag, offset, size) in &indx_entries {
        let mut tag_bytes = [0u8; 4];
        for (i, b) in tag.bytes().take(4).enumerate() {
            tag_bytes[i] = b;
        }
        indx_data.extend_from_slice(&tag_bytes);
        indx_data.extend_from_slice(&(*offset as u64).to_le_bytes());
        indx_data.extend_from_slice(&(*size as u64).to_le_bytes());
    }
    sections_bytes.extend(write_section_with_flags("INDX", &indx_data, 0));

    // Patch header: section_count and indx_offset
    let section_count = indx_entries.len() + 1; // +1 for INDX
    header[80..84].copy_from_slice(&(section_count as u32).to_le_bytes());
    header[88..96].copy_from_slice(&(indx_offset as u64).to_le_bytes());

    let pkg_flags = u32::from_le_bytes(header[84..88].try_into().unwrap());
    if encrypting {
        header[84..88].copy_from_slice(&(pkg_flags | PKG_FLAG_ENCRYPTED).to_le_bytes());
    } else {
        header[84..88].copy_from_slice(&(pkg_flags & !PKG_FLAG_ENCRYPTED).to_le_bytes());
    }

    let mut result = header;
    result.extend(sections_bytes);
    Ok(result)
}

pub fn write_section_with_flags(tag: &str, data: &[u8], flags: u32) -> Vec<u8> {
    let mut tag_bytes = [0u8; 4];
    for (i, b) in tag.bytes().take(4).enumerate() {
        tag_bytes[i] = b;
    }
    let mut out = Vec::with_capacity(SECTION_HEADER_SIZE + data.len());
    out.extend_from_slice(&tag_bytes);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(data);
    out
}

// ─── generic raw rebuild (add/remove sections, patch flags) ──────────────────

/// Rebuild package bytes: drop sections in `drop_tags`, append `extra_sections`
/// before INDX (preserving all other section data and flags), then set/clear
/// package header flags.
pub fn rebuild_raw(
    raw: &[u8],
    drop_tags: &[&str],
    extra_sections: &[(&str, Vec<u8>, u32)],
    set_pkg_flags: u32,
    clear_pkg_flags: u32,
) -> Result<Vec<u8>> {
    let pkg = parse_bytes(raw.to_vec())?;
    let mut header = pkg.raw[..HEADER_SIZE].to_vec();

    let mut sections_bytes: Vec<u8> = Vec::new();
    let mut indx_entries: Vec<(String, usize, usize)> = Vec::new();
    let mut pos = HEADER_SIZE;

    let emit = |tag: &str,
                data: &[u8],
                flags: u32,
                sections_bytes: &mut Vec<u8>,
                indx_entries: &mut Vec<(String, usize, usize)>,
                pos: &mut usize| {
        let data_offset = *pos + SECTION_HEADER_SIZE;
        indx_entries.push((tag.to_string(), data_offset, data.len()));
        sections_bytes.extend(write_section_with_flags(tag, data, flags));
        *pos += SECTION_HEADER_SIZE + data.len();
    };

    for sec in &pkg.sections {
        if sec.tag == "INDX" || drop_tags.contains(&sec.tag.as_str()) {
            continue;
        }
        let data = &pkg.raw[sec.offset..sec.offset + sec.size];
        emit(
            &sec.tag,
            data,
            sec.flags,
            &mut sections_bytes,
            &mut indx_entries,
            &mut pos,
        );
    }
    for (tag, data, flags) in extra_sections {
        emit(
            tag,
            data,
            *flags,
            &mut sections_bytes,
            &mut indx_entries,
            &mut pos,
        );
    }

    let indx_offset = pos;
    let mut indx_data = (indx_entries.len() as u32).to_le_bytes().to_vec();
    for (tag, offset, size) in &indx_entries {
        let mut tag_bytes = [0u8; 4];
        for (i, b) in tag.bytes().take(4).enumerate() {
            tag_bytes[i] = b;
        }
        indx_data.extend_from_slice(&tag_bytes);
        indx_data.extend_from_slice(&(*offset as u64).to_le_bytes());
        indx_data.extend_from_slice(&(*size as u64).to_le_bytes());
    }
    sections_bytes.extend(write_section_with_flags("INDX", &indx_data, 0));

    let section_count = indx_entries.len() + 1;
    header[80..84].copy_from_slice(&(section_count as u32).to_le_bytes());
    header[88..96].copy_from_slice(&(indx_offset as u64).to_le_bytes());
    let pkg_flags = u32::from_le_bytes(header[84..88].try_into().unwrap());
    header[84..88].copy_from_slice(&((pkg_flags | set_pkg_flags) & !clear_pkg_flags).to_le_bytes());

    let mut result = header;
    result.extend(sections_bytes);
    Ok(result)
}

// ─── sealed packages ─────────────────────────────────────────────────────────
//
// A sealed package is opaque at rest and immutable without the author's key:
//   - content sections are AES-256-GCM encrypted with a key derived from a
//     random salt stored in the SEAL section (obfuscation: the runtime can
//     always open it, `strings`/editors cannot);
//   - the whole package is Ed25519-signed; the runtime REFUSES to load a
//     sealed package whose signature is missing or invalid, so any tampering
//     requires the author's private key to re-sign.
// This is weights-grade protection, not DRM: a determined attacker with the
// source code can recover the content. Modification protection, however, is
// real cryptography (Ed25519).

/// SEAL section data: [version u8 = 1][reserved u8][salt 16B]
pub const SEAL_DATA_LEN: usize = 18;
pub const SEAL_VERSION: u8 = 1;

pub fn build_seal_data(salt: &[u8; 16]) -> Vec<u8> {
    let mut data = Vec::with_capacity(SEAL_DATA_LEN);
    data.push(SEAL_VERSION);
    data.push(0);
    data.extend_from_slice(salt);
    data
}

/// Derive the sealing key from the embedded salt and the package name.
pub fn seal_key(salt: &[u8; 16], pkg_name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"aipk-seal-v1");
    hasher.update(salt);
    hasher.update(pkg_name.as_bytes());
    hasher.finalize().into()
}

pub fn is_sealed(raw: &[u8]) -> bool {
    header_flags(raw) & PKG_FLAG_SEALED != 0
}

/// Open a sealed package in memory. Verifies the Ed25519 signature first and
/// refuses tampered or unsigned sealed packages, then decrypts the content
/// sections with the salt-derived key.
pub fn unseal_raw(raw: Vec<u8>) -> Result<Vec<u8>> {
    let pkg = parse_bytes(raw.clone())?;

    crate::cmd::sign::verify_sig_bytes(&raw).map_err(|e| {
        anyhow!(
            "sealed package '{}' failed signature verification — refusing to load ({e})",
            pkg.name
        )
    })?;

    let seal_sec = pkg
        .section("SEAL")
        .ok_or_else(|| anyhow!("sealed flag set but SEAL section missing"))?;
    let data = pkg.section_data(seal_sec);
    if data.len() < SEAL_DATA_LEN || data[0] != SEAL_VERSION {
        anyhow::bail!(
            "unsupported SEAL section (version {})",
            data.first().unwrap_or(&0)
        );
    }
    let salt: [u8; 16] = data[2..18].try_into().unwrap();
    let key = seal_key(&salt, &pkg.name);
    decrypt_pkg_raw(raw, &key)
}

/// Central package loader used by every runtime entry point (serve/run/mcp/…).
/// Handles sealed packages (verify + unseal) and passphrase-encrypted ones.
pub fn load_package(path: &std::path::Path, passphrase: Option<&str>) -> Result<AipkPackage> {
    let raw = std::fs::read(path).map_err(|e| anyhow!("cannot read {}: {e}", path.display()))?;
    if is_sealed(&raw) {
        return parse_bytes(unseal_raw(raw)?);
    }
    if let Some(pw) = passphrase {
        if is_encrypted(&raw) {
            return parse_bytes(decrypt_pkg_with_passphrase(raw, pw)?);
        }
    }
    parse_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::AipkBuilder;

    /// Fixed 32-byte key for AES-GCM primitive tests that don't care about the
    /// passphrase KDF — Argon2id is deliberately slow, so we don't run it just
    /// to get bytes for tests that only exercise `encrypt_section`/`decrypt_section`.
    fn test_key(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn make_pkg(name: &str) -> Vec<u8> {
        let mut b = AipkBuilder::new(name);
        b.add(
            "META",
            format!("[package]\nname = \"{name}\"\n").into_bytes(),
        );
        b.add("PERS", b"You are a secret persona.".to_vec());
        b.build()
    }

    #[test]
    fn derive_key_argon2_is_deterministic_for_same_salt() {
        let salt = [7u8; PKDF_SALT_LEN];
        let params = KdfParams::default();
        let k1 = derive_key_argon2("hunter2", &salt, params).unwrap();
        let k2 = derive_key_argon2("hunter2", &salt, params).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn derive_key_argon2_differs_for_different_passphrases() {
        let salt = [7u8; PKDF_SALT_LEN];
        let params = KdfParams::default();
        let k1 = derive_key_argon2("secret", &salt, params).unwrap();
        let k2 = derive_key_argon2("password", &salt, params).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn derive_key_argon2_differs_for_different_salts() {
        let params = KdfParams::default();
        let k1 = derive_key_argon2("same-passphrase", &[1u8; PKDF_SALT_LEN], params).unwrap();
        let k2 = derive_key_argon2("same-passphrase", &[2u8; PKDF_SALT_LEN], params).unwrap();
        assert_ne!(
            k1, k2,
            "a fresh salt must change the key even for the same passphrase"
        );
    }

    #[test]
    fn pkdf_section_roundtrips_through_bytes() {
        let salt = random_salt();
        let params = KdfParams::default();
        let data = build_pkdf_section(&salt, params);
        let (parsed_salt, parsed_params) = parse_pkdf_section(&data).unwrap();
        assert_eq!(parsed_salt, salt);
        assert_eq!(parsed_params.m_cost_kib, params.m_cost_kib);
        assert_eq!(parsed_params.t_cost, params.t_cost);
        assert_eq!(parsed_params.p_cost, params.p_cost);
    }

    #[test]
    fn encrypt_decrypt_section_roundtrip() {
        let key = test_key(0x42);
        let plaintext = b"Hello, AIPK!";
        let encrypted = encrypt_section(&key, "PERS", plaintext).unwrap();
        assert_ne!(&encrypted, plaintext);
        assert!(encrypted.len() > NONCE_SIZE);

        let decrypted = decrypt_section(&key, "PERS", &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_section_fails_with_wrong_key() {
        let key1 = test_key(0x01);
        let key2 = test_key(0x02);
        let encrypted = encrypt_section(&key1, "PERS", b"secret").unwrap();
        assert!(decrypt_section(&key2, "PERS", &encrypted).is_err());
    }

    #[test]
    fn decrypt_section_fails_with_wrong_tag_aad() {
        let key = test_key(0x03);
        // Encrypt as PERS, try to decrypt as KNOW — AAD mismatch
        let encrypted = encrypt_section(&key, "PERS", b"data").unwrap();
        assert!(decrypt_section(&key, "KNOW", &encrypted).is_err());
    }

    #[test]
    fn nonce_is_random_each_call() {
        let key = test_key(0x04);
        let enc1 = encrypt_section(&key, "PERS", b"data").unwrap();
        let enc2 = encrypt_section(&key, "PERS", b"data").unwrap();
        // Different nonces → different ciphertexts (same plaintext)
        assert_ne!(enc1, enc2);
    }

    #[test]
    fn passphrase_pkg_roundtrip_restores_content_and_drops_pkdf() {
        let raw = make_pkg("pw-test");
        let encrypted =
            encrypt_pkg_with_passphrase(&raw, "correct horse battery staple", &[]).unwrap();

        let pkg = parse_bytes(encrypted.clone()).unwrap();
        assert!(
            pkg.section("PKDF").is_some(),
            "PKDF section must be embedded"
        );
        assert!(is_encrypted(&encrypted));

        let decrypted =
            decrypt_pkg_with_passphrase(encrypted, "correct horse battery staple").unwrap();
        let pkg = parse_bytes(decrypted).unwrap();
        assert!(
            pkg.section("PKDF").is_none(),
            "PKDF section should be dropped once decrypted"
        );
        assert_eq!(pkg.persona().unwrap(), "You are a secret persona.");
    }

    #[test]
    fn passphrase_pkg_decrypt_fails_with_wrong_passphrase() {
        let raw = make_pkg("pw-wrong-test");
        let encrypted = encrypt_pkg_with_passphrase(&raw, "right-passphrase", &[]).unwrap();
        assert!(decrypt_pkg_with_passphrase(encrypted, "wrong-passphrase").is_err());
    }

    #[test]
    fn same_passphrase_encrypts_differently_each_time() {
        let raw = make_pkg("pw-salt-test");
        let enc1 = encrypt_pkg_with_passphrase(&raw, "same-passphrase", &[]).unwrap();
        let enc2 = encrypt_pkg_with_passphrase(&raw, "same-passphrase", &[]).unwrap();

        let pkg1 = parse_bytes(enc1).unwrap();
        let pkg2 = parse_bytes(enc2).unwrap();
        let (salt1, _) =
            parse_pkdf_section(pkg1.section_data(pkg1.section("PKDF").unwrap())).unwrap();
        let (salt2, _) =
            parse_pkdf_section(pkg2.section_data(pkg2.section("PKDF").unwrap())).unwrap();
        assert_ne!(
            salt1, salt2,
            "each encryption must draw a fresh random salt"
        );
    }
}
