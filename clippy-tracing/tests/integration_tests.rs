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
fn check_fix(text: &str, path: &str) {
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
    remove_file(path).unwrap();
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
    check_fix(expected, &path);
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
    check_fix(expected, &path);
}

#[test]
fn fix_one() {
    const GIVEN: &str = "fn main() { }";
    const EXPECTED: &str = "#[tracing::instrument(level = \"trace\", skip())]\nfn main() { }";
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
    let path = setup(GIVEN);
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
fn strip_one() {
    const GIVEN: &str = "#[tracing::instrument(level = \"trace\", skip())]\nfn main() { }";
    const EXPECTED: &str = "fn main() { }";
    strip(GIVEN, EXPECTED);
}
