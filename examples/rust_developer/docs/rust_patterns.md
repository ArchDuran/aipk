# Idiomatic Rust Patterns

## Builder Pattern
```rust
#[derive(Default)]
pub struct ServerConfig {
    host: String,
    port: u16,
    workers: usize,
}

impl ServerConfig {
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into(); self
    }
    pub fn port(mut self, port: u16) -> Self {
        self.port = port; self
    }
}

// Usage
let config = ServerConfig::default()
    .host("localhost")
    .port(8080);
```

## Newtype Pattern
```rust
struct UserId(u64);
struct OrderId(u64);
// Prevents mixing up IDs at compile time
```

## State Machine with Enums
```rust
enum Connection {
    Disconnected,
    Connecting { addr: String },
    Connected { socket: TcpStream },
    Failed { error: io::Error },
}
```

## Iterator Adapters
```rust
// Prefer iterator chains over loops
let total: u32 = items.iter()
    .filter(|x| x.is_active)
    .map(|x| x.value)
    .sum();
```

## Smart Pointers
- `Box<T>` — heap allocation, single owner
- `Rc<T>` — reference counted, single-threaded
- `Arc<T>` — atomic ref count, multi-threaded
- `Mutex<T>` / `RwLock<T>` — interior mutability with locking
- `Cell<T>` / `RefCell<T>` — interior mutability, single-threaded

## Zero-Cost Abstractions
Rust's iterators, closures, and generics compile down to the same machine code as hand-written loops. Use them freely.

## Trait Objects vs Generics
```rust
// Generics (monomorphized, faster, larger binary)
fn process<T: Display>(item: T) { ... }

// Trait objects (dynamic dispatch, smaller binary)
fn process(item: &dyn Display) { ... }
fn process(item: Box<dyn Display>) { ... }
```

## Avoid These Anti-patterns
- `clone()` everywhere — suggests ownership issues
- `unwrap()` / `expect()` in production — use proper error handling
- `unsafe` without clear justification and documentation
- `String` when `&str` suffices in function parameters
- `Vec` return when iterator would do
