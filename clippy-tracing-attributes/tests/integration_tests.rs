#[test]
fn simple() {
    #[clippy_tracing_attributes::clippy_tracing_skip]
    fn add(lhs: i32, rhs: i32) -> i32 {
        lhs + rhs
    }
    assert_eq!(add(1, 1), 2);
}
