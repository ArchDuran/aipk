---
name: error-handling
trigger: Result
---

# Rust Error Handling

## The ? Operator
```rust
use std::fs;
use std::io;

fn read_config(path: &str) -> Result<String, io::Error> {
    let content = fs::read_to_string(path)?;  // returns early on error
    Ok(content)
}
```

## Custom Error Types

### Simple (thiserror)
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Parse error: {0}")]
    Parse(#[from] std::num::ParseIntError),
    
    #[error("Config missing field: {field}")]
    MissingField { field: String },
}
```

### Application-level (anyhow)
```rust
use anyhow::{Context, Result};

fn load(path: &str) -> Result<Config> {
    let s = std::fs::read_to_string(path)
        .context("Failed to read config file")?;
    let config: Config = toml::from_str(&s)
        .context("Failed to parse config")?;
    Ok(config)
}
```

## Rules
- Library crates: use `thiserror` for typed errors
- Binary crates: use `anyhow` for convenience
- Never use `unwrap()` in library code
- Use `expect("reason")` only in tests or where panic is truly unrecoverable
- Prefer `?` over `match` for propagation

## Option → Result
```rust
let val = map.get("key")
    .ok_or(AppError::MissingField { field: "key".into() })?;
```
