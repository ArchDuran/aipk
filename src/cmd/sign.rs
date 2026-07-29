use anyhow::{bail, Result};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ── Key storage ───────────────────────────────────────────────────────────────

fn keys_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".aipk").join("keys")
}

// ── PEM helpers (no extra crate) ──────────────────────────────────────────────

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | if c.len() > 1 { (c[1] as u32) << 8 } else { 0 }
            | if c.len() > 2 { c[2] as u32 } else { 0 };
        out.push(T[((n >> 18) & 63) as usize]);
        out.push(T[((n >> 12) & 63) as usize]);
        out.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize]
        } else {
            b'='
        });
        out.push(if c.len() > 2 {
            T[(n & 63) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).unwrap()
}

fn b64_val(b: u8) -> Result<u8> {
    Ok(match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => 0,
        _ => bail!("invalid base64 byte: {b}"),
    })
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    let s: Vec<u8> = s.bytes().filter(|b| !b" \t\r\n".contains(b)).collect();
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for c in s.chunks(4) {
        if c.len() < 4 {
            bail!("truncated base64 input");
        }
        let n = (b64_val(c[0])? as u32) << 18
            | (b64_val(c[1])? as u32) << 12
            | (b64_val(c[2])? as u32) << 6
            | b64_val(c[3])? as u32;
        out.push((n >> 16) as u8);
        if c[2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if c[3] != b'=' {
            out.push(n as u8);
        }
    }
    Ok(out)
}

fn encode_pem(label: &str, data: &[u8]) -> String {
    let b64 = b64_encode(data);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn decode_pem(label: &str, pem: &str) -> Result<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let body = pem
        .lines()
        .skip_while(|l| *l != begin.as_str())
        .skip(1)
        .take_while(|l| *l != end.as_str())
        .collect::<Vec<_>>()
        .join("");
    if body.is_empty() {
        bail!("PEM label '{label}' not found");
    }
    b64_decode(&body)
}

// ── Fingerprint ───────────────────────────────────────────────────────────────

pub(crate) fn fingerprint(pubkey: &[u8]) -> String {
    let hash = Sha256::digest(pubkey);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    // SSH-style: first 20 hex chars in groups of 4
    hex.chars()
        .take(20)
        .collect::<String>()
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(":")
}

// ── Key I/O ───────────────────────────────────────────────────────────────────

pub fn load_signing_key(path: &Path) -> Result<SigningKey> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read key {}: {e}", path.display()))?;
    let seed = decode_pem("AIPK PRIVATE KEY", &pem)?;
    if seed.len() != 32 {
        bail!("invalid private key length: {} (expected 32)", seed.len());
    }
    let bytes: [u8; 32] = seed.try_into().unwrap();
    Ok(SigningKey::from_bytes(&bytes))
}

fn load_verifying_key(path: &Path) -> Result<VerifyingKey> {
    let pem = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read key {}: {e}", path.display()))?;
    let raw = decode_pem("AIPK PUBLIC KEY", &pem)?;
    if raw.len() != 32 {
        bail!("invalid public key length: {} (expected 32)", raw.len());
    }
    let bytes: [u8; 32] = raw.try_into().unwrap();
    VerifyingKey::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("Invalid public key: {e}"))
}

// ── SIGN section layout (DATA only) ──────────────────────────────────────────
// [0..31]  32 bytes  Ed25519 public key
// [32..95] 64 bytes  Ed25519 signature over (header + all sections up to SIGN)
// [96]      1 byte   Algorithm version: 0x01 = Ed25519
// [97]      1 byte   Reserved
// Total DATA: 98 bytes

const SIGN_DATA_LEN: usize = 98;
const ALGO_ED25519: u8 = 0x01;

fn build_sign_section(pubkey: &[u8; 32], sig: &[u8; 64]) -> Vec<u8> {
    let mut data = Vec::with_capacity(SIGN_DATA_LEN);
    data.extend_from_slice(pubkey);
    data.extend_from_slice(sig);
    data.push(ALGO_ED25519);
    data.push(0x00); // reserved
    assert_eq!(data.len(), SIGN_DATA_LEN);

    let mut section = Vec::with_capacity(crate::format::SECTION_HEADER_SIZE + SIGN_DATA_LEN);
    section.extend_from_slice(b"SIGN");
    section.extend_from_slice(&0u32.to_le_bytes()); // section flags
    section.extend_from_slice(&(SIGN_DATA_LEN as u64).to_le_bytes());
    section.extend_from_slice(&data);
    section
}

// ── Byte-level sign / verify (shared with `aipk seal`) ───────────────────────

/// Set the SIGNED flag, sign the bytes, and append the SIGN section.
/// The input must not already contain a SIGN section.
pub fn sign_bytes(mut bytes: Vec<u8>, signing_key: &SigningKey) -> Vec<u8> {
    let flags = u32::from_le_bytes(bytes[84..88].try_into().unwrap());
    bytes[84..88].copy_from_slice(&(flags | crate::format::PKG_FLAG_SIGNED).to_le_bytes());

    let signature = signing_key.sign(&bytes);
    let pubkey_bytes: [u8; 32] = *signing_key.verifying_key().as_bytes();
    let sig_bytes: [u8; 64] = signature.to_bytes();
    bytes.extend(build_sign_section(&pubkey_bytes, &sig_bytes));
    bytes
}

/// Verify the SIGN section over `bytes`. Returns the signer's public key on
/// success; errors when the section is missing, malformed, or invalid.
pub fn verify_sig_bytes(bytes: &[u8]) -> Result<[u8; 32]> {
    let pkg = crate::format::parse_bytes(bytes.to_vec())?;
    let sign_sec = pkg
        .section("SIGN")
        .ok_or_else(|| anyhow::anyhow!("no SIGN section — package is not signed"))?;

    let data = pkg.section_data(sign_sec);
    if data.len() < 96 {
        bail!("SIGN section too small ({} bytes, need ≥ 96)", data.len());
    }
    let pubkey_bytes: [u8; 32] = data[0..32].try_into().unwrap();
    let sig_bytes: [u8; 64] = data[32..96].try_into().unwrap();

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("malformed public key in SIGN section: {e}"))?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);

    let sign_sec_start = sign_sec.offset - crate::format::SECTION_HEADER_SIZE;
    verifying_key
        .verify(&bytes[..sign_sec_start], &signature)
        .map_err(|e| anyhow::anyhow!("signature invalid: {e}"))?;
    Ok(pubkey_bytes)
}

/// One-line signature status for a trust-decision prompt (MCP launch consent,
/// etc). Deliberately does NOT say "trusted" or "safe" — a valid signature
/// only proves the package bytes match what *some* key signed (integrity);
/// it says nothing about who holds that key (identity) or whether the user
/// should trust them (a separate decision this function doesn't make).
pub fn describe_signature(pkg: &crate::format::AipkPackage) -> String {
    match verify_sig_bytes(&pkg.raw) {
        Ok(pubkey) => format!(
            "integrity OK, unknown publisher (fingerprint SHA256:{})",
            fingerprint(&pubkey)
        ),
        Err(_) => "UNSIGNED — origin cannot be verified".to_string(),
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Generate an Ed25519 keypair for package signing.
pub fn keygen(name: &str, force: bool) -> Result<()> {
    let dir = keys_dir();
    std::fs::create_dir_all(&dir)?;

    let priv_path = dir.join(format!("{name}.pem"));
    let pub_path = dir.join(format!("{name}.pub"));

    if priv_path.exists() && !force {
        bail!(
            "Key '{}' already exists: {}\nUse --force to overwrite.",
            name,
            priv_path.display()
        );
    }

    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    let priv_pem = encode_pem("AIPK PRIVATE KEY", &signing_key.to_bytes());
    let pub_pem = encode_pem("AIPK PUBLIC KEY", verifying_key.as_bytes());

    // Private key: mode 0600 on Unix
    write_private_key(&priv_path, &priv_pem)?;
    std::fs::write(&pub_path, pub_pem.as_bytes())?;

    let fp = fingerprint(verifying_key.as_bytes());
    println!("Generated Ed25519 keypair '{name}'");
    println!("  private : {}", priv_path.display());
    println!("  public  : {}", pub_path.display());
    println!("  fingerprint: SHA256:{fp}");
    println!();
    println!(
        "To sign a package:   aipk sign pkg.aipk --key {}",
        priv_path.display()
    );
    println!("To verify:           aipk verify-sig pkg.aipk");
    Ok(())
}

/// Sign a .aipk package with an Ed25519 private key.
pub fn sign(pkg_path: &Path, key_path: &Path, output: Option<&Path>) -> Result<()> {
    let signing_key = load_signing_key(key_path)?;
    let verifying_key = signing_key.verifying_key();

    let mut bytes = std::fs::read(pkg_path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", pkg_path.display()))?;

    if bytes.len() < crate::format::HEADER_SIZE {
        bail!("Not a valid .aipk file");
    }
    if &bytes[..4] != b"AIPK" {
        bail!("Not a valid .aipk file (bad magic)");
    }

    // Check if already signed
    let pkg = crate::format::parse_bytes(bytes.clone())?;
    if pkg.section("SIGN").is_some() {
        bail!(
            "Package is already signed.\n\
             Extract, modify, rebuild, then sign the new package."
        );
    }

    // Set SIGNED flag (bit 2) in header bytes[84..88]
    let flags = u32::from_le_bytes(bytes[84..88].try_into().unwrap());
    bytes[84..88].copy_from_slice(&(flags | 0x04).to_le_bytes());

    // Sign all bytes as they stand (no SIGN section yet)
    let signature = signing_key.sign(&bytes);

    // Append SIGN section
    let pubkey_bytes: [u8; 32] = *verifying_key.as_bytes();
    let sig_bytes: [u8; 64] = signature.to_bytes();
    bytes.extend(build_sign_section(&pubkey_bytes, &sig_bytes));

    let out_path = output.unwrap_or(pkg_path);
    std::fs::write(out_path, &bytes)
        .map_err(|e| anyhow::anyhow!("Cannot write {}: {e}", out_path.display()))?;

    let fp = fingerprint(&pubkey_bytes);
    println!("Signed: {}", out_path.display());
    println!("  algorithm  : Ed25519");
    println!("  fingerprint: SHA256:{fp}");
    println!(
        "  size delta : +{} bytes (SIGN section)",
        crate::format::SECTION_HEADER_SIZE + SIGN_DATA_LEN
    );
    Ok(())
}

/// Verify an Ed25519 signature on a .aipk package.
/// Exit behaviour: Ok = valid, Err = invalid or missing.
pub fn verify_sig(pkg_path: &Path, expected_key: Option<&Path>) -> Result<()> {
    let bytes = std::fs::read(pkg_path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", pkg_path.display()))?;

    let pkg = crate::format::parse_bytes(bytes.clone())?;

    let sign_sec = pkg
        .section("SIGN")
        .ok_or_else(|| anyhow::anyhow!("No SIGN section — package is not signed"))?;

    let data = pkg.section_data(sign_sec);
    if data.len() < 96 {
        bail!("SIGN section too small ({} bytes, need ≥ 96)", data.len());
    }

    let pubkey_bytes: [u8; 32] = data[0..32].try_into().unwrap();
    let sig_bytes: [u8; 64] = data[32..96].try_into().unwrap();

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Malformed public key in SIGN section: {e}"))?;
    let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);

    // Optionally check the key matches an expected public key
    if let Some(kp) = expected_key {
        let expected = load_verifying_key(kp)?;
        if expected.as_bytes() != verifying_key.as_bytes() {
            bail!(
                "Key mismatch: package was signed by a different key\n  \
                 package key : SHA256:{}\n  \
                 expected key: SHA256:{}",
                fingerprint(verifying_key.as_bytes()),
                fingerprint(expected.as_bytes()),
            );
        }
    }

    // signed content = everything before the SIGN section header
    let sign_sec_start = sign_sec.offset - crate::format::SECTION_HEADER_SIZE;
    let signed_content = &bytes[..sign_sec_start];

    verifying_key
        .verify(signed_content, &signature)
        .map_err(|e| anyhow::anyhow!("Signature invalid: {e}"))?;

    let fp = fingerprint(&pubkey_bytes);
    println!("Signature valid");
    println!("  algorithm  : Ed25519");
    println!("  fingerprint: SHA256:{fp}");
    println!("  signed bytes: {}", signed_content.len());
    Ok(())
}

// ── Platform helpers ──────────────────────────────────────────────────────────

#[cfg(unix)]
fn write_private_key(path: &Path, pem: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(pem.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private_key(path: &Path, pem: &str) -> Result<()> {
    std::fs::write(path, pem.as_bytes())?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn make_minimal_aipk(name: &str) -> Vec<u8> {
        crate::format::AipkBuilder::new(name).build()
    }

    #[test]
    fn b64_roundtrip() {
        for data in [b"".as_ref(), b"a", b"ab", b"abc", b"hello world!"] {
            let enc = b64_encode(data);
            assert_eq!(b64_decode(&enc).unwrap(), data);
        }
    }

    #[test]
    fn pem_roundtrip() {
        let data = b"test key data 12345678901234";
        let pem = encode_pem("TEST LABEL", data);
        assert!(pem.starts_with("-----BEGIN TEST LABEL-----"));
        assert_eq!(decode_pem("TEST LABEL", &pem).unwrap(), data);
    }

    #[test]
    fn fingerprint_is_stable() {
        let pubkey = [0u8; 32];
        let fp = fingerprint(&pubkey);
        // SHA-256 of 32 zero bytes: 66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925
        assert!(fp.starts_with("6668:"));
        assert_eq!(fp.len(), "6668:7aad:f862:bd77:6c8f".len());
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let pkg_bytes = make_minimal_aipk("test-pkg");

        // Generate key
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        // Apply SIGNED flag and sign
        let mut to_sign = pkg_bytes.clone();
        let flags = u32::from_le_bytes(to_sign[84..88].try_into().unwrap());
        to_sign[84..88].copy_from_slice(&(flags | 0x04).to_le_bytes());

        let signature = signing_key.sign(&to_sign);

        let pubkey_bytes: [u8; 32] = *verifying_key.as_bytes();
        let sig_bytes: [u8; 64] = signature.to_bytes();
        to_sign.extend(build_sign_section(&pubkey_bytes, &sig_bytes));

        // Parse and verify
        let pkg = crate::format::parse_bytes(to_sign.clone()).unwrap();
        let sign_sec = pkg.section("SIGN").expect("SIGN section missing");
        let data = pkg.section_data(sign_sec);

        let pk: [u8; 32] = data[0..32].try_into().unwrap();
        let s: [u8; 64] = data[32..96].try_into().unwrap();
        let vk = VerifyingKey::from_bytes(&pk).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&s);

        let sign_start = sign_sec.offset - crate::format::SECTION_HEADER_SIZE;
        vk.verify(&to_sign[..sign_start], &sig)
            .expect("verification failed");
    }

    #[test]
    fn tamper_detection() {
        let pkg_bytes = make_minimal_aipk("tamper-test");

        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        let mut to_sign = pkg_bytes.clone();
        let flags = u32::from_le_bytes(to_sign[84..88].try_into().unwrap());
        to_sign[84..88].copy_from_slice(&(flags | 0x04).to_le_bytes());

        let signature = signing_key.sign(&to_sign);
        let pubkey_bytes: [u8; 32] = *verifying_key.as_bytes();
        let sig_bytes: [u8; 64] = signature.to_bytes();
        to_sign.extend(build_sign_section(&pubkey_bytes, &sig_bytes));

        // Tamper: flip a byte in the package name area
        to_sign[8] ^= 0xFF;

        let pkg = crate::format::parse_bytes(to_sign.clone()).unwrap();
        let sign_sec = pkg.section("SIGN").unwrap();
        let data = pkg.section_data(sign_sec);

        let pk: [u8; 32] = data[0..32].try_into().unwrap();
        let s: [u8; 64] = data[32..96].try_into().unwrap();
        let vk = VerifyingKey::from_bytes(&pk).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&s);

        let sign_start = sign_sec.offset - crate::format::SECTION_HEADER_SIZE;
        assert!(vk.verify(&to_sign[..sign_start], &sig).is_err());
    }

    #[test]
    fn describe_signature_reports_unsigned_for_plain_package() {
        let pkg_bytes = make_minimal_aipk("no-sig");
        let pkg = crate::format::parse_bytes(pkg_bytes).unwrap();
        assert_eq!(
            describe_signature(&pkg),
            "UNSIGNED — origin cannot be verified"
        );
    }

    #[test]
    fn describe_signature_reports_integrity_ok_for_signed_package() {
        let pkg_bytes = make_minimal_aipk("signed-pkg");
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();

        let mut to_sign = pkg_bytes.clone();
        let flags = u32::from_le_bytes(to_sign[84..88].try_into().unwrap());
        to_sign[84..88].copy_from_slice(&(flags | 0x04).to_le_bytes());
        let signature = signing_key.sign(&to_sign);
        to_sign.extend(build_sign_section(
            verifying_key.as_bytes(),
            &signature.to_bytes(),
        ));

        let pkg = crate::format::parse_bytes(to_sign).unwrap();
        let desc = describe_signature(&pkg);
        assert!(desc.starts_with("integrity OK, unknown publisher (fingerprint SHA256:"));
        assert!(!desc.to_lowercase().contains("trusted"));
        assert!(!desc.to_lowercase().contains("safe"));
    }

    #[test]
    fn sign_section_data_layout() {
        let pubkey = [0xAAu8; 32];
        let sig = [0xBBu8; 64];
        let section = build_sign_section(&pubkey, &sig);

        // header: 4 tag + 4 flags + 8 size
        assert_eq!(&section[0..4], b"SIGN");
        let size = u64::from_le_bytes(section[8..16].try_into().unwrap()) as usize;
        assert_eq!(size, SIGN_DATA_LEN);

        let data = &section[crate::format::SECTION_HEADER_SIZE..];
        assert_eq!(&data[0..32], &[0xAAu8; 32]);
        assert_eq!(&data[32..96], &[0xBBu8; 64]);
        assert_eq!(data[96], ALGO_ED25519);
        assert_eq!(data[97], 0x00);
    }
}
