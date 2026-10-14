//! Import a ShareX custom uploader (`.sxcu`).
//!
//! Header values that look like secrets are stored in the secret store and
//! the config keeps a `keyring:` reference instead of the value.

use std::path::Path;

use serde_json::Value;
use snapper_core::settings::{HeaderSetting, UploadConfig};

use crate::secrets::SecretStore;
use crate::UploadError;

/// Read `path` and return an [`UploadConfig::Http`] target.
pub fn import_sxcu(
    path: &Path,
    secrets: &(dyn SecretStore + Sync),
) -> Result<UploadConfig, UploadError> {
    let text = std::fs::read_to_string(path).map_err(|err| UploadError::Config(err.to_string()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| UploadError::Config(err.to_string()))?;
    let name = value
        .get("Name")
        .and_then(|item| item.as_str())
        .unwrap_or("custom");
    let id = slug(name);
    let url = value
        .get("RequestURL")
        .and_then(|item| item.as_str())
        .ok_or_else(|| UploadError::Config("sxcu file has no RequestURL".to_owned()))?
        .to_owned();
    let method = value
        .get("RequestMethod")
        .and_then(|item| item.as_str())
        .unwrap_or("POST")
        .to_owned();
    let file_field = value
        .get("FileFormName")
        .and_then(|item| item.as_str())
        .unwrap_or("file")
        .to_owned();
    let mut headers = Vec::new();
    if let Some(map) = value.get("Headers").and_then(|item| item.as_object()) {
        for (name, header_value) in map {
            let Some(raw) = header_value.as_str() else {
                continue;
            };
            let stored = if secret_header(name, raw) {
                let account = format!("http:{id}:{name}");
                secrets.set(&account, raw)?;
                format!("keyring:{account}")
            } else {
                raw.to_owned()
            };
            headers.push(HeaderSetting {
                name: name.clone(),
                value: stored,
            });
        }
    }
    let url_json_pointer = value
        .get("URL")
        .and_then(|item| item.as_str())
        .map(sharex_pointer)
        .unwrap_or_default();
    let delete_url_json_pointer = value
        .get("DeletionURL")
        .and_then(|item| item.as_str())
        .map(sharex_pointer)
        .unwrap_or_default();
    Ok(UploadConfig::Http {
        id,
        url,
        method,
        file_field,
        headers,
        url_json_pointer,
        delete_url_json_pointer,
    })
}

fn secret_header(name: &str, value: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("authorization")
        || name.contains("token")
        || name.contains("secret")
        || name.contains("api-key")
        || name.contains("apikey")
        || value.to_ascii_lowercase().starts_with("bearer ")
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "custom".to_owned()
    } else {
        out.to_owned()
    }
}

/// `$json:data.link$` becomes `/data/link`. A value that is already a pointer is kept.
pub fn sharex_pointer(pattern: &str) -> String {
    let trimmed = pattern.trim();
    if let Some(inner) = trimmed
        .strip_prefix("$json:")
        .and_then(|rest| rest.strip_suffix('$'))
    {
        let path = inner.split('|').next().unwrap_or(inner);
        return dots_to_pointer(path);
    }
    if trimmed.starts_with('/') {
        return trimmed.to_owned();
    }
    dots_to_pointer(trimmed)
}

fn dots_to_pointer(path: &str) -> String {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return String::new();
    }
    let mut pointer = String::new();
    for part in path.split(['.', '/']) {
        if part.is_empty() {
            continue;
        }
        pointer.push('/');
        pointer.push_str(part);
    }
    pointer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySecretStore;

    #[test]
    fn sxcu_import_keeps_the_bearer_token_out_of_config() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("example.sxcu");
        std::fs::write(
            &path,
            r#"{
                "Name": "Example host",
                "DestinationType": "ImageUploader",
                "RequestMethod": "POST",
                "RequestURL": "https://example.com/upload",
                "FileFormName": "file",
                "Headers": { "Authorization": "Bearer super-secret", "Accept": "application/json" },
                "URL": "$json:data.url$",
                "DeletionURL": "$json:data.delete_url$"
            }"#,
        )
        .expect("write");
        let secrets = MemorySecretStore::default();
        let config = import_sxcu(&path, &secrets).expect("import");
        let text = toml::to_string(&config).expect("toml");
        assert!(!text.contains("super-secret"), "{text}");
        assert!(!text.contains("Bearer"), "{text}");
        let UploadConfig::Http {
            id,
            url,
            file_field,
            headers,
            url_json_pointer,
            delete_url_json_pointer,
            ..
        } = config
        else {
            panic!("http config");
        };
        assert_eq!(id, "example-host");
        assert_eq!(url, "https://example.com/upload");
        assert_eq!(file_field, "file");
        assert_eq!(url_json_pointer, "/data/url");
        assert_eq!(delete_url_json_pointer, "/data/delete_url");
        let auth = headers.iter().find(|header| header.name == "Authorization");
        assert_eq!(
            auth.map(|header| header.value.as_str()),
            Some("keyring:http:example-host:Authorization")
        );
        assert_eq!(
            secrets
                .get("http:example-host:Authorization")
                .expect("secret"),
            "Bearer super-secret"
        );
        let accept = headers
            .iter()
            .find(|header| header.name == "Accept")
            .unwrap();
        assert_eq!(accept.value, "application/json");
    }

    #[test]
    fn sharex_json_syntax_becomes_a_pointer() {
        assert_eq!(sharex_pointer("$json:data.link$"), "/data/link");
        assert_eq!(sharex_pointer("/data/link"), "/data/link");
        assert_eq!(sharex_pointer("$json:url|raw$"), "/url");
    }
}
