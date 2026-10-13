//! Crash-log behaviour of the `snapper-cli` binary.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("snapper-log-{nanos}"));
        fs::create_dir_all(&path).expect("create temp log dir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn panic_writes_a_crash_file_and_a_log_without_image_bytes() {
    let dir = TempDir::new();
    let bin = env!("CARGO_BIN_EXE_snapper-cli");
    // Present in the environment only. A report that dumped the process
    // environment, or a screenshot buffer, would copy this marker.
    let marker = "PNG_IMAGE_BYTES_NOT_A_LOG_iVBORw0KGgo";

    let crashed = Command::new(bin)
        .env("SNAPPER_LOG_DIR", &dir.0)
        .env("SNAPPER_TEST_PANIC", "1")
        .env("SNAPPER_FAKE_IMAGE", marker)
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn snapper-cli");
    assert!(
        !crashed.status.success(),
        "expected the test panic to fail the process: {crashed:?}"
    );

    let mut crash_text = None;
    let mut saw_log = false;
    for entry in fs::read_dir(&dir.0).expect("read log dir") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = fs::read(entry.path()).expect("read file");
        let text = String::from_utf8(bytes).unwrap_or_else(|err| {
            panic!("{name} is not UTF-8 text, so it may contain image bytes: {err}")
        });
        assert!(
            !text.contains(marker),
            "{name} contained the image marker:\n{text}"
        );
        if name.starts_with("crash-") && name.ends_with(".txt") {
            crash_text = Some(text);
        } else if name.starts_with("snapper-cli.log") {
            saw_log = true;
            assert!(
                text.contains("logging initialised"),
                "log file {name} was:\n{text}"
            );
        }
    }

    let crash_text = crash_text.expect("crash report was not written");
    assert!(crash_text.contains("SNAPPER_TEST_PANIC"), "{crash_text}");
    assert!(crash_text.contains("version:"), "{crash_text}");
    assert!(crash_text.contains(std::env::consts::OS), "{crash_text}");
    assert!(crash_text.contains("backtrace:"), "{crash_text}");
    assert!(saw_log, "daily log file was not created");

    let printed = Command::new(bin)
        .env("SNAPPER_LOG_DIR", &dir.0)
        .env_remove("SNAPPER_TEST_PANIC")
        .env_remove("RUST_LOG")
        .arg("--print-log-dir")
        .output()
        .expect("spawn print-log-dir");
    assert!(printed.status.success(), "{printed:?}");
    let stdout = String::from_utf8(printed.stdout).expect("utf-8 path");
    assert_eq!(stdout.trim(), dir.0.display().to_string());
}
