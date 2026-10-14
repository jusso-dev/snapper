//! Append-only upload history at `data_dir/history.jsonl`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::UploadError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub time: String,
    pub file: String,
    pub url: String,
}

/// Append one JSON line. Creates the file and its parent directory.
pub fn append_history(path: &Path, entry: &HistoryEntry) -> Result<(), UploadError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| UploadError::Config(err.to_string()))?;
        }
    }
    let mut line =
        serde_json::to_string(entry).map_err(|err| UploadError::Config(err.to_string()))?;
    line.push('\n');
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| UploadError::Config(err.to_string()))?;
    file.write_all(line.as_bytes())
        .map_err(|err| UploadError::Config(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_appends_json_lines() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("history.jsonl");
        let first = HistoryEntry {
            time: "2026-01-02T03:04:05+11:00".to_owned(),
            file: "shot.png".to_owned(),
            url: "https://cdn.example/shot.png".to_owned(),
        };
        let second = HistoryEntry {
            time: "2026-01-02T03:05:05+11:00".to_owned(),
            file: "other.png".to_owned(),
            url: "https://cdn.example/other.png".to_owned(),
        };
        append_history(&path, &first).expect("first");
        append_history(&path, &second).expect("second");
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: HistoryEntry = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(parsed, first);
        let parsed: HistoryEntry = serde_json::from_str(lines[1]).expect("json");
        assert_eq!(parsed.url, second.url);
    }
}
