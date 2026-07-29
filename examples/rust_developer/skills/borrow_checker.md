---
name: borrow-checker
trigger: borrow
---

# Borrow Checker Guide

## Core Rules
1. At any given time, you can have EITHER one mutable reference OR any number of immutable references
2. References must always be valid (no dangling pointers)
3. Mutable references are exclusive — only one at a time

## Common Errors and Fixes

### E0502 — cannot borrow as mutable because it is also borrowed as immutable
```rust
// Wrong
let mut v = vec![1, 2, 3];
let first = &v[0];       // immutable borrow starts
v.push(4);               // ERROR: mutable borrow
println!("{}", first);   // immutable borrow used here

// Fix: end immutable borrow first
let mut v = vec![1, 2, 3];
let first_val = v[0];    // copy the value, not a reference
v.push(4);               // ok now
```

### E0505 — cannot move out because it is borrowed
```rust
// Wrong
let s = String::from("hello");
let r = &s;
drop(s);            // ERROR: can't move while borrowed
println!("{}", r);

// Fix: use r before dropping s, or clone
```

### E0499 — cannot borrow as mutable more than once
```rust
// Wrong
let mut v = vec![1, 2, 3];
let a = &mut v;
let b = &mut v;   // ERROR

// Fix: use indices, or use split_at_mut()
let (left, right) = v.split_at_mut(1);
```

## Lifetime Annotations
Needed when the compiler can't infer how long references live:
```rust
// This function returns a reference — which lifetime?
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
```

## Key Mental Model
The borrow checker enforces at compile time what a garbage collector does at runtime.
If the checker rejects code, ask: "Could this cause a use-after-free or data race?"
Usually the answer is yes, and the fix is to restructure ownership.
