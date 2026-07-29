//! `aipk seal` / `aipk unseal` — author-locked packages.
//!
//! Sealing = encrypt content sections with a package-embedded salt key
//! (opaque at rest, like model weights) + mandatory Ed25519 signature
//! (immutable without the author's private key: the runtime refuses sealed
//! packages that fail verification). Unsealing back to an editable package
//! requires the same private key that sealed it.

use anyhow::{bail, Result};
use rand::RngCore;
use std::path::Path;

use crate::cmd::sign::{load_signing_key, sign_bytes, verify_sig_bytes};
use crate::crypto::{
    build_seal_data, encrypt_pkg_raw, is_encrypted, is_sealed, rebuild_raw, seal_key, unseal_raw,
};
use crate::format::{parse_bytes, PKG_FLAG_SEALED, PKG_FLAG_SIGNED};

pub fn seal(pkg_path: &Path, key_path: &Path, output: Option<&Path>) -> Result<()> {
    let raw = std::fs::read(pkg_path)?;
    let pkg = parse_bytes(raw.clone())?;

    if is_sealed(&raw) {
        bail!("Package is already sealed.");
    }
    if pkg.section("SIGN").is_some() {
        bail!("Package is already signed — sealing performs its own signing. Use the unsigned package.");
    }
    if is_encrypted(&raw) {
        bail!("Package is passphrase-encrypted — decrypt it first (`aipk decrypt`), then seal.");
    }

    let signing_key = load_signing_key(key_path)?;

    // 1. Encrypt content sections with a salt-derived key
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let key = seal_key(&salt, &pkg.name);
    let encrypted = encrypt_pkg_raw(&raw, &key, &[])?;

    // 2. Embed the salt (SEAL section) and set the SEALED flag
    let with_seal = rebuild_raw(
        &encrypted,
        &[],
        &[("SEAL", build_seal_data(&salt), 0)],
        PKG_FLAG_SEALED,
        0,
    )?;

    // 3. Sign — from here on, any modification invalidates the package
    let signed = sign_bytes(with_seal, &signing_key);

    let out_path = output.unwrap_or(pkg_path);
    std::fs::write(out_path, &signed)?;

    println!("✓ Sealed: {}", out_path.display());
    println!("  content   : AES-256-GCM (opaque without the aipk runtime)");
    println!(
        "  integrity : Ed25519 — modification without your private key is detected and refused"
    );
    println!("  serve/run work as usual; `aipk extract` and `aipk export` are blocked.");
    println!(
        "  To edit again: aipk unseal {} --key <same-private-key>",
        out_path.display()
    );
    Ok(())
}

pub fn unseal(pkg_path: &Path, key_path: &Path, output: Option<&Path>) -> Result<()> {
    let raw = std::fs::read(pkg_path)?;
    if !is_sealed(&raw) {
        bail!("Package is not sealed.");
    }

    // Only the author (same keypair that signed the seal) may unseal.
    let signer_pubkey = verify_sig_bytes(&raw)?;
    let signing_key = load_signing_key(key_path)?;
    if signing_key.verifying_key().as_bytes() != &signer_pubkey {
        bail!("This key did not seal the package — only the author's key can unseal it.");
    }

    let decrypted = unseal_raw(raw)?;
    let plain = rebuild_raw(
        &decrypted,
        &["SEAL", "SIGN"],
        &[],
        0,
        PKG_FLAG_SEALED | PKG_FLAG_SIGNED,
    )?;

    let out_path = output.unwrap_or(pkg_path);
    std::fs::write(out_path, &plain)?;
    println!(
        "✓ Unsealed: {} (plain, editable package)",
        out_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::AipkBuilder;
    use ed25519_dalek::SigningKey;

    fn make_pkg(name: &str) -> Vec<u8> {
        let mut b = AipkBuilder::new(name);
        b.add(
            "META",
            format!("[package]\nname = \"{name}\"\n").into_bytes(),
        );
        b.add("PERS", b"You are a secret persona.".to_vec());
        b.build()
    }

    fn seal_in_memory(raw: &[u8], name: &str, key: &SigningKey) -> Vec<u8> {
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let k = seal_key(&salt, name);
        let enc = encrypt_pkg_raw(raw, &k, &[]).unwrap();
        let with_seal = rebuild_raw(
            &enc,
            &[],
            &[("SEAL", build_seal_data(&salt), 0)],
            PKG_FLAG_SEALED,
            0,
        )
        .unwrap();
        sign_bytes(with_seal, key)
    }

    #[test]
    fn seal_roundtrip_hides_and_restores_content() {
        let raw = make_pkg("sealed-test");
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        let sealed = seal_in_memory(&raw, "sealed-test", &key);

        assert!(is_sealed(&sealed));
        // Persona plaintext must not appear in the sealed bytes
        let hay = sealed
            .windows(b"secret persona".len())
            .any(|w| w == b"secret persona");
        assert!(!hay, "sealed package leaks plaintext");

        // Runtime can open it
        let opened = unseal_raw(sealed).unwrap();
        let pkg = parse_bytes(opened).unwrap();
        assert_eq!(pkg.persona().unwrap(), "You are a secret persona.");
    }

    #[test]
    fn tampered_sealed_package_is_refused() {
        let raw = make_pkg("tamper-seal");
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        let mut sealed = seal_in_memory(&raw, "tamper-seal", &key);

        // Flip one byte in the middle of the encrypted payload
        let mid = sealed.len() / 2;
        sealed[mid] ^= 0xFF;

        assert!(
            unseal_raw(sealed).is_err(),
            "tampered package must not load"
        );
    }

    #[test]
    fn sealed_without_signature_is_refused() {
        let raw = make_pkg("unsigned-seal");
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let k = seal_key(&salt, "unsigned-seal");
        let enc = encrypt_pkg_raw(&raw, &k, &[]).unwrap();
        let with_seal = rebuild_raw(
            &enc,
            &[],
            &[("SEAL", build_seal_data(&salt), 0)],
            PKG_FLAG_SEALED,
            0,
        )
        .unwrap();
        // No SIGN section appended
        assert!(unseal_raw(with_seal).is_err());
    }

    #[test]
    fn seal_key_depends_on_salt_and_name() {
        let salt_a = [1u8; 16];
        let salt_b = [2u8; 16];
        assert_ne!(seal_key(&salt_a, "x"), seal_key(&salt_b, "x"));
        assert_ne!(seal_key(&salt_a, "x"), seal_key(&salt_a, "y"));
        assert_eq!(seal_key(&salt_a, "x"), seal_key(&salt_a, "x"));
    }
}
