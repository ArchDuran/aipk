# Common Rust Compiler Errors

## E0502 — Cannot borrow as mutable because also borrowed as immutable
Cause: trying to mutate while an immutable reference is live.
Fix: end the immutable borrow before mutating, or restructure to avoid overlapping borrows.

## E0505 — Cannot move out of value because it is borrowed
Cause: trying to move/drop a value while a reference to it exists.
Fix: use the reference before moving, or clone the value.

## E0499 — Cannot borrow as mutable more than once at a time
Cause: two mutable references to the same data at once.
Fix: use indices, split_at_mut(), or RefCell<T> for interior mutability.

## E0277 — Trait bound not satisfied
Cause: type doesn't implement a required trait.
Fix: implement the trait, add a derive macro, or use a different type.
Example: `#[derive(Clone, Debug, PartialEq)]`

## E0308 — Mismatched types
Cause: type mismatch — most common with Option/Result unwrapping.
Fix: use ?, unwrap_or(), map(), or proper type conversion.

## E0382 — Use of moved value
Cause: using a value after it was moved into a function or binding.
Fix: clone() if you need the value again, or pass a reference instead.

## E0597 — Value does not live long enough
Cause: a reference outlives the value it points to.
Fix: adjust lifetimes, use owned values, or use Arc<T>.

## E0716 — Temporary value dropped while borrowed
Cause: taking a reference to a temporary that is immediately dropped.
Fix: bind the temporary to a variable first.
```rust
// Wrong
let r = something().method();  // temporary dropped

// Fix
let owned = something();
let r = owned.method();
```
