use std::fs::{remove_file, OpenOptions};
use std::io::{Read, Write};
use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_clippy-tracing");

fn setup(text: &str) -> String {
    let id = uuid::Uuid::new_v4();
    let path = format!("/tmp/{id}.rs");
    let mut file = OpenOptions::new()
        .create(true)
        .read(false)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
    path
}
fn check_file(text: &str, path: &str) {
    let mut file = OpenOptions::new()
        .create(false)
        .read(true)
        .write(false)
        .open(path)
        .unwrap();
    let mut buffer = String::new();
    file.read_to_string(&mut buffer).unwrap();
    println!("path: {path}");
    assert_eq!(text, buffer);
}

fn fix(given: &str, expected: &str) {
    let path = setup(given);
    let output = Command::new(BINARY)
        .args(["--action", "fix", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);
    check_file(expected, &path);
    remove_file(path).unwrap();
}
fn strip(given: &str, expected: &str) {
    let path = setup(given);
    let output = Command::new(BINARY)
        .args(["--action", "strip", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);
    check_file(expected, &path);
    remove_file(path).unwrap();
}

#[test]
fn fix_one() {
    const GIVEN: &str = "fn main() { }\nfn add(lhs: i32, rhs: i32) {\n    lhs + rhs\n}";
    const EXPECTED: &str = "#[tracing::instrument(level = \"trace\", skip())]\nfn main() { }\n#[tracing::instrument(level = \"trace\", skip(lhs, rhs))]\nfn add(lhs: i32, rhs: i32) {\n    lhs + rhs\n}";
    fix(GIVEN, EXPECTED);
}

#[test]
fn check_one() {
    const GIVEN: &str = "fn main() { }";
    let path = setup(GIVEN);
    let output = Command::new(BINARY)
        .args(["--action", "check", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let expected_stdout = format!("Missing instrumentation at {path}:1:0.\n");
    assert_eq!(output.stdout, expected_stdout.as_bytes());
    assert_eq!(output.stderr, []);
    remove_file(path).unwrap();
}

#[test]
fn check_two() {
    const GIVEN: &str = "#[tracing::instrument(level = \"trace\", skip())]\nfn main() { }\n#[test]\nfn my_test() { }";
    let path: String = setup(GIVEN);
    let output = Command::new(BINARY)
        .args(["--action", "check", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);
    remove_file(path).unwrap();
}

#[test]
fn check_three() {
    const GIVEN: &str = "impl One {\n    fn one() { }\n}";
    let path = setup(GIVEN);
    let output = Command::new(BINARY)
        .args(["--action", "check", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let expected_stdout = format!("Missing instrumentation at {path}:2:4.\n");
    assert_eq!(output.stdout, expected_stdout.as_bytes());
    assert_eq!(output.stderr, []);
    remove_file(path).unwrap();
}

#[test]
fn strip_one() {
    const GIVEN: &str = "#[tracing::instrument(level = \"trace\", skip())]\nfn main() { }";
    const EXPECTED: &str = "fn main() { }";
    strip(GIVEN, EXPECTED);
}

#[test]
fn strip_two() {
    const GIVEN: &str =
        "#[tracing::instrument(    \nlevel = \"trace\",\n    skip()\n)]\nfn main() { }";
    const EXPECTED: &str = "fn main() { }";
    strip(GIVEN, EXPECTED);
}

#[test]
fn strip_three() {
    const EXPECTED: &str = "impl Unit {\n    fn one() {}\n}";
    const GIVEN: &str =
        "impl Unit {\n    #[tracing::instrument(level = \"trace\", skip())]\n    fn one() {}\n}";
    strip(GIVEN, EXPECTED);
}

#[test]
fn readme() {
    const GIVEN: &str = r#"fn main() {
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
}"#;
    let path: String = setup(GIVEN);

    // Check
    let output = Command::new(BINARY)
        .args(["--action", "check", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let missing = format!("Missing instrumentation at {path}:9:4.\n");
    assert_eq!(output.stdout, missing.as_bytes());
    assert_eq!(output.stderr, []);

    const EXPECTED: &str = r#"#[tracing::instrument(level = "trace", skip())]
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
}"#;

    // Fix
    let output = Command::new(BINARY)
        .args(["--action", "fix", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);
    check_file(EXPECTED, &path);

    // Check
    let output = Command::new(BINARY)
        .args(["--action", "check", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);

    // Strip
    let output = Command::new(BINARY)
        .args(["--action", "strip", "--path", &path])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, []);
    assert_eq!(output.stderr, []);
    check_file(GIVEN, &path);
}
