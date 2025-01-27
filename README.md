```rust
let mut sigs = Signals::deadly();
println!("Got deadly signal {}", sigs.next().unwrap());
```
