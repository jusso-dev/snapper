//! Argument parsing for `snapper-cli`. Capture itself needs a display.

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_every_command() {
    let mut command = Command::cargo_bin("snapper-cli").expect("bin");
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("capture"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("edit"))
        .stdout(predicate::str::contains("ocr"))
        .stdout(predicate::str::contains("record"));
}

#[test]
fn capture_help_lists_targets() {
    let mut command = Command::cargo_bin("snapper-cli").expect("bin");
    command
        .args(["capture", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("full"))
        .stdout(predicate::str::contains("monitor"))
        .stdout(predicate::str::contains("window"))
        .stdout(predicate::str::contains("region"))
        .stdout(predicate::str::contains("--out"))
        .stdout(predicate::str::contains("--clipboard"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--delay"));
}

#[test]
fn missing_capture_target_is_usage() {
    let mut command = Command::cargo_bin("snapper-cli").expect("bin");
    command.arg("capture").assert().code(2);
}

#[test]
fn list_without_a_subject_is_usage() {
    let mut command = Command::cargo_bin("snapper-cli").expect("bin");
    command.arg("list").assert().code(2);
}
