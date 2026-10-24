//! External `snapper-plugin-<id>` processes.
//!
//! The protocol is documented in `docs/plugins.md`. One JSON object is written
//! to stdin. One JSON line is read from stdout. The process is killed when
//! `timeout` elapses.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{UploadError, UploadResult, UploadTarget};

#[derive(Debug, Clone)]
pub struct ExternalTarget {
    pub path: PathBuf,
    pub timeout: Duration,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct Request<'a> {
    version: u32,
    kind: &'a str,
    file: &'a str,
    mime: &'a str,
    config: &'a serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct Response {
    ok: bool,
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredPlugin {
    pub id: String,
    pub path: PathBuf,
    pub schema: serde_json::Value,
}

#[async_trait]
impl UploadTarget for ExternalTarget {
    fn id(&self) -> &str {
        self.path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("plugin")
    }

    async fn upload(
        &self,
        name: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<UploadResult, UploadError> {
        // The temp file is removed when `file` drops.
        let file = write_temp(name, &bytes)?;
        run_plugin(&self.path, &file, mime, &self.config, self.timeout)
    }
}

/// A unique, owner-only temp file named after `name`.
fn write_temp(name: &str, bytes: &[u8]) -> Result<tempfile::TempPath, UploadError> {
    let safe = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mut file = tempfile::Builder::new()
        .prefix("snapper-upload-")
        .suffix(&format!("-{safe}"))
        .tempfile()
        .map_err(|err| UploadError::Config(err.to_string()))?;
    file.write_all(bytes)
        .map_err(|err| UploadError::Config(err.to_string()))?;
    Ok(file.into_temp_path())
}

/// Spawn `path`, send one request, and read one response line.
pub fn run_plugin(
    path: &Path,
    file: &Path,
    mime: &str,
    config: &serde_json::Value,
    timeout: Duration,
) -> Result<UploadResult, UploadError> {
    let request = Request {
        version: 1,
        kind: "upload",
        file: &file.to_string_lossy(),
        mime,
        config,
    };
    let payload =
        serde_json::to_vec(&request).map_err(|err| UploadError::Protocol(err.to_string()))?;
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| UploadError::Config(format!("could not start {}: {err}", path.display())))?;
    let deadline = Instant::now() + timeout;
    let stdin = child.stdin.take();
    let Some(stdout) = child.stdout.take() else {
        kill(&mut child);
        return Err(UploadError::Protocol("plugin stdout was closed".to_owned()));
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    // The write happens on this thread too, so a plugin that never reads stdin
    // is still covered by the timeout.
    std::thread::spawn(move || {
        let written = match stdin {
            Some(mut stdin) => stdin.write_all(&payload),
            None => Ok(()),
        };
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send((written, line));
    });
    let Ok((written, line)) = receiver.recv_timeout(timeout) else {
        kill(&mut child);
        return Err(UploadError::Timeout);
    };
    wait_until(&mut child, deadline);
    match written {
        Err(err) if line.trim().is_empty() => Err(UploadError::Protocol(err.to_string())),
        _ => parse_response(&line),
    }
}

/// Reap `child`, killing it if it is still running at `deadline`.
fn wait_until(child: &mut Child, deadline: Instant) {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => return kill(child),
        }
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn parse_response(line: &str) -> Result<UploadResult, UploadError> {
    let response: Response = serde_json::from_str(line.trim())
        .map_err(|err| UploadError::Protocol(format!("{err}; plugin said {line:?}")))?;
    if !response.ok {
        return Err(UploadError::Rejected(
            response.error.unwrap_or_else(|| "plugin failed".to_owned()),
        ));
    }
    let url = response.url.ok_or(UploadError::MissingUrl)?;
    Ok(UploadResult {
        url,
        delete_url: None,
    })
}

/// `snapper-plugin-<id>` in the plugins directory or on `PATH`, unless `command` is set.
pub fn resolve_command(id: &str, command: &str) -> Result<PathBuf, UploadError> {
    if !command.is_empty() {
        return Ok(PathBuf::from(command));
    }
    let name = format!("snapper-plugin-{id}");
    let installed = snapper_core::settings::data_dir()
        .join("plugins")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if installed.is_file() {
        return Ok(installed);
    }
    if let Some(path) = lookup_path(&name) {
        return Ok(path);
    }
    Err(UploadError::Config(format!(
        "could not find {name} on PATH or in the plugins directory"
    )))
}

fn lookup_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Plugins in `dir` whose names start with `snapper-plugin-`.
pub fn discover_in(dir: &Path) -> Vec<DiscoveredPlugin> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_prefix("snapper-plugin-") else {
            continue;
        };
        if stem.is_empty() || !path.is_file() {
            continue;
        }
        let id = stem
            .strip_suffix(std::env::consts::EXE_SUFFIX)
            .unwrap_or(stem)
            .to_owned();
        let schema = describe(&path).unwrap_or(serde_json::json!({}));
        found.push(DiscoveredPlugin { id, path, schema });
    }
    found.sort_by(|left, right| left.id.cmp(&right.id));
    found
}

/// Plugins in the data directory and on `PATH`.
pub fn discover() -> Vec<DiscoveredPlugin> {
    let mut found = discover_in(&snapper_core::settings::data_dir().join("plugins"));
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for plugin in discover_in(&dir) {
                if !found.iter().any(|existing| existing.id == plugin.id) {
                    found.push(plugin);
                }
            }
        }
    }
    found
}

/// Run `path --describe`. A plugin that does not answer within a few seconds is killed.
pub fn describe(path: &Path) -> Result<serde_json::Value, UploadError> {
    let mut child = Command::new(path)
        .arg("--describe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| UploadError::Config(err.to_string()))?;
    let Some(mut stdout) = child.stdout.take() else {
        kill(&mut child);
        return Err(UploadError::Protocol("plugin stdout was closed".to_owned()));
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        let _ = sender.send(output);
    });
    let timeout = Duration::from_secs(5);
    let Ok(output) = receiver.recv_timeout(timeout) else {
        kill(&mut child);
        return Err(UploadError::Timeout);
    };
    wait_until(&mut child, Instant::now() + Duration::from_secs(1));
    serde_json::from_slice(&output).map_err(|err| UploadError::Protocol(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_binary() -> PathBuf {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let status = Command::new(cargo)
            .args(["build", "-p", "snapper-plugin-echo", "--quiet"])
            .current_dir(&root)
            .status()
            .expect("build the echo plugin");
        assert!(status.success(), "echo plugin failed to build");
        let mut path = root
            .join("target")
            .join("debug")
            .join("snapper-plugin-echo");
        if cfg!(windows) {
            path.set_extension("exe");
        }
        assert!(path.exists(), "missing {}", path.display());
        path
    }

    #[test]
    fn external_echo_returns_a_file_url() {
        let bin = plugin_binary();
        let dir = tempfile::tempdir().expect("temp");
        let file = dir.path().join("shot.png");
        std::fs::write(&file, b"png").expect("write");
        let result = run_plugin(
            &bin,
            &file,
            "image/png",
            &serde_json::json!({}),
            Duration::from_secs(10),
        )
        .expect("plugin");
        assert!(result.url.starts_with("file://"), "{}", result.url);
        assert!(result.url.contains("shot.png"), "{}", result.url);
    }

    #[test]
    fn external_malformed_output_is_an_error() {
        let bin = plugin_binary();
        let dir = tempfile::tempdir().expect("temp");
        let file = dir.path().join("shot.png");
        std::fs::write(&file, b"png").expect("write");
        let err = run_plugin(
            &bin,
            &file,
            "image/png",
            &serde_json::json!({"mode": "bad"}),
            Duration::from_secs(10),
        )
        .expect_err("malformed");
        assert!(matches!(err, UploadError::Protocol(_)), "{err}");
    }

    #[test]
    fn external_timeout_kills_the_process() {
        let bin = plugin_binary();
        let dir = tempfile::tempdir().expect("temp");
        let file = dir.path().join("shot.png");
        std::fs::write(&file, b"png").expect("write");
        let err = run_plugin(
            &bin,
            &file,
            "image/png",
            &serde_json::json!({"mode": "sleep"}),
            Duration::from_millis(200),
        )
        .expect_err("timeout");
        assert!(matches!(err, UploadError::Timeout), "{err}");
    }

    #[test]
    fn external_plugin_is_discovered_from_a_directory() {
        let bin = plugin_binary();
        let dir = tempfile::tempdir().expect("temp");
        let dest = dir.path().join("snapper-plugin-echo");
        std::fs::copy(&bin, &dest).expect("copy");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&dest).expect("meta").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&dest, permissions).expect("chmod");
        }
        let found = discover_in(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "echo");
        assert!(
            found[0].schema.get("title").is_some(),
            "{:?}",
            found[0].schema
        );
    }
}
