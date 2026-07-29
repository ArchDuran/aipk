use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::crypto::{decrypt_pkg_with_passphrase, encrypt_pkg_with_passphrase, is_encrypted};

/// Encrypt content sections of a .aipk package.
pub fn encrypt(
    pkg_path: &Path,
    passphrase: &str,
    section_filter: &[String],
    output: Option<&Path>,
) -> Result<()> {
    let raw = fs::read(pkg_path)?;

    if is_encrypted(&raw) {
        anyhow::bail!(
            "{} is already encrypted. Decrypt it first with `aipk decrypt`.",
            pkg_path.display()
        );
    }

    let encrypted = encrypt_pkg_with_passphrase(&raw, passphrase, section_filter)?;

    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| pkg_path.to_path_buf());

    fs::write(&out_path, &encrypted)?;

    let size = encrypted.len();
    println!(
        "✓ Encrypted {} ({:.1} KB)",
        out_path.display(),
        size as f64 / 1024.0
    );
    if section_filter.is_empty() {
        println!("  Sections: PERS, KNOW, SKIL, CLMS, CLMV, SRCS, IDTY, ANSP, PLCY, NKNW, TOOL, THKG, TEST");
    } else {
        println!("  Sections: {}", section_filter.join(", "));
    }
    println!("  Passphrase required to serve or run this package.");
    Ok(())
}

/// Decrypt a previously encrypted .aipk package.
pub fn decrypt(pkg_path: &Path, passphrase: &str, output: Option<&Path>) -> Result<()> {
    let raw = fs::read(pkg_path)?;

    if !is_encrypted(&raw) {
        anyhow::bail!("{} does not appear to be encrypted.", pkg_path.display());
    }

    let decrypted = decrypt_pkg_with_passphrase(raw, passphrase)?;

    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| pkg_path.to_path_buf());

    fs::write(&out_path, &decrypted)?;

    let size = decrypted.len();
    println!(
        "✓ Decrypted {} ({:.1} KB)",
        out_path.display(),
        size as f64 / 1024.0
    );
    Ok(())
}
