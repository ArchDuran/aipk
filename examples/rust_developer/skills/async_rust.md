---
name: async-rust
trigger: async
---

# Async Rust

## Tokio Basics
```rust
use tokio;

#[tokio::main]
async fn main() {
    let result = fetch_data().await;
    println!("{result}");
}

async fn fetch_data() -> String {
    // async functions return a Future
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    "data".to_string()
}
```

## Concurrent Tasks
```rust
use tokio::task;

// Spawn independent tasks
let h1 = task::spawn(async { fetch_a().await });
let h2 = task::spawn(async { fetch_b().await });
let (a, b) = tokio::join!(h1, h2);  // wait for both

// Or with select! — first one wins
tokio::select! {
    result = fetch_a() => println!("A finished: {result}"),
    result = fetch_b() => println!("B finished: {result}"),
}
```

## Common Patterns

### Async trait (stable since Rust 1.75)
```rust
trait Fetcher {
    async fn fetch(&self, url: &str) -> Result<String>;
}
```

### Timeout
```rust
use tokio::time::{timeout, Duration};

let result = timeout(Duration::from_secs(5), fetch_data()).await
    .map_err(|_| AppError::Timeout)?;
```

### Channel
```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(32);

tokio::spawn(async move {
    tx.send("hello").await.unwrap();
});

while let Some(msg) = rx.recv().await {
    println!("{msg}");
}
```

## Runtime Choice
- `tokio` — most common, batteries included
- `async-std` — std-like API
- `smol` — minimal
- Single-threaded: `#[tokio::main(flavor = "current_thread")]`
