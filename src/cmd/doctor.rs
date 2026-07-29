//! `aipk doctor` — sanity-check the environment before a new user hits a
//! confusing error three commands in. Probes the same backend candidates
//! `up.rs` uses (reused, not reimplemented), and checks write access to the
//! current directory, since "no inference backend" and "can't write here"
//! are the two most common sources of new-user confusion independent of
//! anything AIPK-specific.

use crate::cmd::up::{list_models, CANDIDATES};
use anyhow::Result;
use std::io::Write;

pub async fn run(llm_url: Option<String>) -> Result<()> {
    let mut ok = true;
    println!("AIPK environment check\n");

    // ── Backend ──────────────────────────────────────────────────────────────
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    match llm_url {
        Some(url) => match list_models(&client, &url).await {
            Ok(models) if !models.is_empty() => {
                println!(
                    "✓ Backend at {url} reachable, {} model(s) reported",
                    models.len()
                );
            }
            Ok(_) => {
                println!("! Backend at {url} reachable but reports no models — pull one");
                ok = false;
            }
            Err(e) => {
                println!("✗ Backend at {url} unreachable: {e}");
                ok = false;
            }
        },
        None => {
            let mut found = false;
            for (url, name) in CANDIDATES {
                match list_models(&client, url).await {
                    Ok(models) if !models.is_empty() => {
                        println!(
                            "✓ Found {name} at {url}, {} model(s) reported",
                            models.len()
                        );
                        found = true;
                        break;
                    }
                    Ok(_) => {
                        println!("! Found {name} at {url} but it reports no models");
                        found = true;
                        break;
                    }
                    Err(_) => continue,
                }
            }
            if !found {
                println!(
                    "✗ No inference backend found. Probed: {}.\n  Start one (e.g. `ollama serve`) or pass --llm-url.",
                    CANDIDATES.iter().map(|(u, n)| format!("{n} ({u})")).collect::<Vec<_>>().join(", ")
                );
                ok = false;
            }
        }
    }

    // ── Write access to the current directory ──────────────────────────────
    let probe_path = std::env::current_dir()?.join(".aipk-doctor-probe");
    match std::fs::File::create(&probe_path).and_then(|mut f| f.write_all(b"ok")) {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe_path);
            println!("✓ Current directory is writable");
        }
        Err(e) => {
            println!("✗ Cannot write to current directory: {e}");
            ok = false;
        }
    }

    println!();
    if ok {
        println!("All checks passed. Try: aipk init my-package && cd my-package && aipk add-docs ./some-dir");
    } else {
        println!("Some checks failed — fix the items above before `aipk build`/`aipk serve`.");
        anyhow::bail!("environment check failed");
    }
    Ok(())
}
