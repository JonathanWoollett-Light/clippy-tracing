# clippy-tracing

[![Crates.io](https://img.shields.io/crates/v/clippy-tracing)](https://crates.io/crates/clippy-tracing)
[![codecov](https://codecov.io/gh/JonathanWoollett-Light/clippy-tracing/branch/master/graph/badge.svg?token=II1xtnbCDX)](https://codecov.io/gh/JonathanWoollett-Light/clippy-tracing)

**This is rough tool**

A tool to add, remove and check for `tracing::instrument` in large projects where it is infeasible to manually add it to thousands of functions.

### Installation

```
cargo install clippy-tracing
```

### Usage


*This is tested in the [`readme()` integration test](clippy-tracing/tests/integration_tests.rs#L98-L176).*

```rust
fn main() {
    println!("Hello World!");
}
fn add(lhs: i32, rhs: i32) -> i32 {
    lhs + rhs
}
#[cfg(tests)]
mod tests {
    fn sub(lhs: i32, rhs: i32) -> i32 {
        lhs - rhs
    }
    #[test]
    fn test_one() {
        assert_eq!(add(1,1), sub(2, 1));
    }
}
```

```bash
clippy-tracing --action check
echo $? # 1
clippy-tracing --action fix
echo $? # 0
```

```rust
#[tracing::instrument(level = "trace", skip())]
fn main() {
    println!("Hello World!");
}
#[tracing::instrument(level = "trace", skip(lhs, rhs))]
fn add(lhs: i32, rhs: i32) -> i32 {
    lhs + rhs
}
#[cfg(tests)]
mod tests {
    #[tracing::instrument(level = "trace", skip(lhs, rhs))]
    fn sub(lhs: i32, rhs: i32) -> i32 {
        lhs - rhs
    }
    #[test]
    fn test_one() {
        assert_eq!(add(1,1), sub(2, 1));
    }
}
```

```bash
clippy-tracing --action check
echo $? # 0
clippy-tracing --action strip
echo $? # 0
```

```rust
fn main() {
    println!("Hello World!");
}
fn add(lhs: i32, rhs: i32) -> i32 {
    lhs + rhs
}
#[cfg(tests)]
mod tests {
    fn sub(lhs: i32, rhs: i32) -> i32 {
        lhs - rhs
    }
    #[test]
    fn test_one() {
        assert_eq!(add(1,1), sub(2, 1));
    }
}
```