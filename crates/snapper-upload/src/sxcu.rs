//! Import a ShareX custom uploader (`.sxcu`).
//!
//! Header values that look like secrets are stored in the secret store and
//! the config keeps a `keyring:` reference instead of the value.

use std::path::Path;

use serde_json::Value;
use snapper_core::settings::{HeaderSetting, UploadConfig};

use crate::secrets::SecretStore;
use crate::UploadError;

/// Read `path` and return an [`UploadConfig::Http`] target whose id is not in `taken`.
pub fn import_sxcu(
    path: &Path,
    secrets: &(dyn SecretStore + Sync),
    taken: &[&str],
) -> Result<UploadConfig, UploadError> {
    let text = std::fs::read_to_string(path).map_err(|err| UploadError::Config(err.to_string()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| UploadError::Config(err.to_string()))?;
    let name = value
        .get("Name")
        .and_then(|item| item.as_str())
        .unwrap_or("custom");
    let id = unique_id(&slug(name), taken);
    for field in ["Arguments", "Parameters"] {
        if value
            .get(field)
            .and_then(|item| item.as_object())
            .is_some_and(|map| !map.is_empty())
        {
            return Err(UploadError::Config(format!(
                "sxcu {field} are not supported yet"
            )));
        }
    }
    if let Some(body) = value.get("Body").and_then(|item| item.as_str()) {
        if body != "MultipartFormData" {
            return Err(UploadError::Config(format!(
                "sxcu Body {body} is not supported; only MultipartFormData is"
            )));
        }
    }
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
    let url_json_pointer = value
        .get("URL")
        .and_then(|item| item.as_str())
        .map(sharex_pointer)
        .transpose()?
        .unwrap_or_default();
    // A deletion URL is optional, so an unsupported template is dropped
    // rather than failing the import.
    let delete_url_json_pointer = value
        .get("DeletionURL")
        .and_then(|item| item.as_str())
        .and_then(|pattern| sharex_pointer(pattern).ok())
        .unwrap_or_default();
    let mut pending = Vec::new();
    if let Some(map) = value.get("Headers").and_then(|item| item.as_object()) {
        for (name, header_value) in map {
            let Some(raw) = header_value.as_str() else {
                continue;
            };
            if raw.starts_with("keyring:") {
                return Err(UploadError::Config(format!(
                    "header {name} refers to a keychain entry"
                )));
            }
            pending.push((name.clone(), raw.to_owned()));
        }
    }
    // Secrets are stored last so a rejected file leaves nothing in the keychain.
    let mut headers = Vec::new();
    for (name, raw) in pending {
        let value = if secret_header(&name, &raw) {
            let account = format!("http:{id}:{name}");
            secrets.set(&account, &raw)?;
            format!("keyring:{account}")
        } else {
            raw
        };
        headers.push(HeaderSetting { name, value });
    }
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
        || name.contains("api_key")
        || name.contains("cookie")
        || name.contains("password")
        || value.to_ascii_lowercase().starts_with("bearer ")
}

/// `base`, or `base-2`, `base-3`, ... if `base` is in `taken`.
pub fn unique_id(base: &str, taken: &[&str]) -> String {
    let mut id = base.to_owned();
    let mut n = 2;
    while taken.contains(&id.as_str()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
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

/// `$json:data.link$` and `{json:data.link}` become `/data/link`; `data[0]`
/// becomes `/data/0`. An empty pointer, from an empty value or `{response}`,
/// means the whole response body. A value that is already a pointer is kept.
pub fn sharex_pointer(pattern: &str) -> Result<String, UploadError> {
    let trimmed = pattern.trim();
    if matches!(trimmed, "" | "{response}" | "$response$") {
        return Ok(String::new());
    }
    let inner = trimmed
        .strip_prefix("$json:")
        .and_then(|rest| rest.strip_suffix('$'))
        .or_else(|| {
            trimmed
                .strip_prefix("{json:")
                .and_then(|rest| rest.strip_suffix('}'))
        });
    if let Some(inner) = inner {
        if !inner.contains(['$', '{', '}']) {
            let path = inner.split('|').next().unwrap_or(inner);
            return Ok(dots_to_pointer(path));
        }
    } else if trimmed.starts_with('/') {
        return Ok(trimmed.to_owned());
    } else if !trimmed.contains(['$', '{', '}', ':']) {
        return Ok(dots_to_pointer(trimmed));
    }
    Err(UploadError::Config(format!(
        "URL template {trimmed:?} is not supported; use a single {{json:...}} value"
    )))
}

fn dots_to_pointer(path: &str) -> String {
    let path = path.replace('[', ".").replace(']', "");
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
        let config = import_sxcu(&path, &secrets, &[]).expect("import");
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
        let pointer = |pattern| sharex_pointer(pattern).expect(pattern);
        assert_eq!(pointer("$json:data.link$"), "/data/link");
        assert_eq!(pointer("{json:data.link}"), "/data/link");
        assert_eq!(pointer("{json:files[0].url}"), "/files/0/url");
        assert_eq!(pointer("/data/link"), "/data/link");
        assert_eq!(pointer("$json:url|raw$"), "/url");
        assert_eq!(pointer("{response}"), "");
        assert!(sharex_pointer("https://i.host/{json:id}").is_err());
    }

    fn write_sxcu(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        let path = dir.path().join("example.sxcu");
        std::fs::write(&path, body).expect("write");
        path
    }

    #[test]
    fn sxcu_import_rejects_a_keyring_reference_in_a_header() {
        let dir = tempfile::tempdir().expect("temp");
        let path = write_sxcu(
            &dir,
            r#"{ "Name": "x", "RequestURL": "https://evil.example/", "Headers": { "X-Note": "keyring:s3:prod" } }"#,
        );
        let secrets = MemorySecretStore::default();
        assert!(import_sxcu(&path, &secrets, &[]).is_err());
    }

    #[test]
    fn sxcu_import_does_not_reuse_an_existing_id() {
        let dir = tempfile::tempdir().expect("temp");
        let path = write_sxcu(
            &dir,
            r#"{ "Name": "Example host", "RequestURL": "https://example.com/", "Headers": { "Authorization": "Bearer new" } }"#,
        );
        let secrets = MemorySecretStore::default();
        secrets
            .set("http:example-host:Authorization", "Bearer old")
            .expect("store");
        let config = import_sxcu(&path, &secrets, &["example-host"]).expect("import");
        assert_eq!(config.id(), "example-host-2");
        assert_eq!(
            secrets
                .get("http:example-host:Authorization")
                .expect("secret"),
            "Bearer old"
        );
    }

    #[test]
    fn sxcu_import_drops_an_unsupported_deletion_url() {
        let dir = tempfile::tempdir().expect("temp");
        let path = write_sxcu(
            &dir,
            r#"{ "Name": "x", "RequestURL": "https://example.com/", "URL": "{json:url}", "DeletionURL": "https://example.com/delete/{json:id}" }"#,
        );
        let secrets = MemorySecretStore::default();
        let UploadConfig::Http {
            url_json_pointer,
            delete_url_json_pointer,
            ..
        } = import_sxcu(&path, &secrets, &[]).expect("import")
        else {
            panic!("http config");
        };
        assert_eq!(url_json_pointer, "/url");
        assert_eq!(delete_url_json_pointer, "");
    }

    #[test]
    fn sxcu_import_rejects_an_unsupported_url_without_storing_secrets() {
        let dir = tempfile::tempdir().expect("temp");
        let path = write_sxcu(
            &dir,
            r#"{ "Name": "x", "RequestURL": "https://example.com/", "URL": "https://i.example.com/{json:id}", "Headers": { "Authorization": "Bearer secret" } }"#,
        );
        let secrets = MemorySecretStore::default();
        assert!(import_sxcu(&path, &secrets, &[]).is_err());
        assert!(secrets.get("http:x:Authorization").is_err());
    }

    #[test]
    fn sxcu_import_keeps_cookies_out_of_config() {
        let dir = tempfile::tempdir().expect("temp");
        let path = write_sxcu(
            &dir,
            r#"{ "Name": "x", "RequestURL": "https://example.com/", "Headers": { "Cookie": "session=hunter2", "X-Api-Key": "k-123" } }"#,
        );
        let secrets = MemorySecretStore::default();
        let config = import_sxcu(&path, &secrets, &[]).expect("import");
        let text = toml::to_string(&config).expect("toml");
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("k-123"), "{text}");
        assert_eq!(
            secrets.get("http:x:Cookie").expect("secret"),
            "session=hunter2"
        );
    }

    #[test]
    fn unique_id_skips_taken_ids() {
        assert_eq!(unique_id("s3", &[]), "s3");
        assert_eq!(unique_id("s3", &["s3", "s3-2"]), "s3-3");
    }
}
