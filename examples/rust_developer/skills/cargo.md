---
name: cargo
trigger: cargo
---

# Cargo Patterns

## Essential Commands
```bash
cargo new my_project          # new binary
cargo new --lib my_lib        # new library
cargo build                   # debug build
cargo build --release         # optimized build
cargo run                     # build + run
cargo test                    # run all tests
cargo check                   # fast type-check (no binary)
cargo clippy                  # linter
cargo fmt                     # formatter
cargo doc --open              # generate + open docs
cargo add serde               # add dependency (cargo-edit)
cargo update                  # update Cargo.lock
```

## Cargo.toml Patterns

### Features
```toml
[features]
default = ["std"]
std = []
async = ["tokio"]

[dependencies]
tokio = { version = "1", features = ["full"], optional = true }
```

### Workspace
```toml
[workspace]
members = ["crates/core", "crates/cli", "crates/server"]
resolver = "2"
```

### Profile Optimization
```toml
[profile.release]
lto = true          # link-time optimization
codegen-units = 1   # better optimization, slower compile
strip = true        # strip symbols from binary
```

## Dependency Best Practices
- Pin exact versions for binaries: `serde = "=1.0.197"`
- Use `~` for libraries: `serde = "~1.0"`
- Check `cargo audit` for vulnerabilities
- Use `cargo deny` for license compliance

## Useful Cargo Plugins
```bash
cargo install cargo-edit      # cargo add/rm/upgrade
cargo install cargo-watch     # cargo watch -x run
cargo install cargo-audit     # security audit
cargo install cargo-expand    # expand macros
cargo install cargo-flamegraph # profiling
```
