# clippy-tracing

[![Crates.io](https://img.shields.io/crates/v/clippy-tracing)](https://crates.io/crates/clippy-tracing)
[![codecov](https://codecov.io/gh/JonathanWoollett-Light/clippy-tracing/branch/master/graph/badge.svg?token=II1xtnbCDX)](https://codecov.io/gh/JonathanWoollett-Light/clippy-tracing)

**This is rough tool**

A tool to add, remove and check for `tracing::instrument` in large projects where it is infeasible to manually add it to thousands of functions.

### Installation

```
cargo install clippy-tracing
```

### Examples

- `clippy-tracing --action check`
- `clippy-tracing --action fix`
- `clippy-tracing --action strip`
- `clippy-tracing --action check path --path /path/to/my/file.rs`
- `clippy-tracing --action fix path --path /path/to/my/file.rs`
- `clippy-tracing --action strip path --path /path/to/my/file.rs`
- `clippy-tracing --action check text -- text "$(cat /home/jonathan/Projects/clippy-tracing/src/test.rs)"`
- `clippy-tracing --action fix text -- text "$(cat /home/jonathan/Projects/clippy-tracing/src/test.rs)"`
- `clippy-tracing --action strip text -- text "$(cat /home/jonathan/Projects/clippy-tracing/src/test.rs)"`