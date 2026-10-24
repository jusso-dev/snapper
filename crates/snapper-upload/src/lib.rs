//! Upload targets. See ADR 0004.
//!
//! Built-in targets are S3-compatible storage (AWS S3 and Cloudflare R2),
//! anonymous Imgur, and a custom HTTP multipart upload. Secrets are read from
//! the OS keychain and are not part of [`snapper_core::settings::UploadConfig`].
//! External programs that speak the JSON protocol live in [`external`].

mod external;
mod history;
mod http_target;
mod imgur;
mod s3;
mod secrets;
mod sxcu;

pub use external::{discover, discover_in, run_plugin, DiscoveredPlugin, ExternalTarget};
pub use history::{append_history, HistoryEntry};
pub use secrets::{Keychain, MemorySecretStore, SecretStore};
pub use sxcu::{import_sxcu, unique_id};

use async_trait::async_trait;
use snapper_core::settings::UploadConfig;

/// Where an upload can be fetched, and an optional delete URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub url: String,
    pub delete_url: Option<String>,
}

/// Failure while uploading or reading a secret.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("upload was rejected: {0}")]
    Rejected(String),
    #[error("upload failed: {0}")]
    Network(String),
    #[error("{0}")]
    Secret(String),
    #[error("the response did not contain a URL")]
    MissingUrl,
    #[error("the plugin timed out")]
    Timeout,
    #[error("the plugin returned malformed output: {0}")]
    Protocol(String),
    #[error("{0}")]
    Config(String),
}

/// An upload destination.
#[async_trait]
pub trait UploadTarget: Send + Sync {
    fn id(&self) -> &str;
    async fn upload(
        &self,
        name: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<UploadResult, UploadError>;
}

/// Build a target from settings. The S3 secret is loaded from `secrets`.
pub async fn upload_with(
    config: &UploadConfig,
    name: &str,
    bytes: Vec<u8>,
    mime: &str,
    secrets: &(dyn SecretStore + Sync),
) -> Result<UploadResult, UploadError> {
    match config {
        UploadConfig::S3 {
            id,
            endpoint,
            bucket,
            region,
            key_prefix,
            public_base_url,
            access_key_id,
        } => {
            let secret = secrets.get(&format!("s3:{id}"))?;
            let target = s3::S3Target {
                id: id.clone(),
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                key_prefix: key_prefix.clone(),
                public_base_url: public_base_url.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key: secret,
            };
            target.upload(name, bytes, mime).await
        }
        UploadConfig::Imgur { id, client_id } => {
            let target = imgur::ImgurTarget::anonymous(id.clone(), client_id.clone());
            target.upload(name, bytes, mime).await
        }
        UploadConfig::Http {
            id,
            url,
            method,
            file_field,
            headers,
            url_json_pointer,
            delete_url_json_pointer,
        } => {
            let target = http_target::HttpTarget {
                id: id.clone(),
                url: url.clone(),
                method: method.clone(),
                file_field: file_field.clone(),
                headers: headers.clone(),
                url_json_pointer: url_json_pointer.clone(),
                delete_url_json_pointer: delete_url_json_pointer.clone(),
                secrets,
            };
            target.upload(name, bytes, mime).await
        }
        UploadConfig::External {
            id,
            command,
            config,
        } => {
            let path = external::resolve_command(id, command)?;
            let value = toml_to_json(&toml::Value::Table(config.clone()));
            let target = ExternalTarget {
                path,
                timeout: std::time::Duration::from_secs(60),
                config: value,
            };
            target.upload(name, bytes, mime).await
        }
    }
}

fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(text) => serde_json::Value::String(text.clone()),
        toml::Value::Integer(number) => serde_json::Value::from(*number),
        toml::Value::Float(number) => serde_json::json!(number),
        toml::Value::Boolean(flag) => serde_json::Value::Bool(*flag),
        toml::Value::Datetime(date) => serde_json::Value::String(date.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(toml_to_json).collect())
        }
        toml::Value::Table(table) => {
            let mut map = serde_json::Map::new();
            for (key, value) in table {
                map.insert(key.clone(), toml_to_json(value));
            }
            serde_json::Value::Object(map)
        }
    }
}

pub(crate) fn http_client() -> Result<reqwest::Client, UploadError> {
    // Redirects are not followed: custom headers can carry keychain secrets,
    // and reqwest only strips `Authorization` and `Cookie` on a cross-host hop.
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| UploadError::Network(err.to_string()))
}

/// Total time allowed for one upload: a minute, plus a second for every 64 KiB,
/// so a large scroll capture on a slow link is not cut off.
pub(crate) fn upload_timeout(bytes: usize) -> std::time::Duration {
    std::time::Duration::from_secs(60 + (bytes / (64 * 1024)) as u64)
}

/// Join a prefix and a file name with single slashes.
pub fn object_key(prefix: &str, name: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let name = name.trim_matches('/');
    if prefix.is_empty() {
        name.to_owned()
    } else if name.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// `{base}/{key}` without a double slash. Each key segment is percent-encoded.
pub fn public_url(base: &str, key: &str) -> String {
    let base = base.trim_end_matches('/');
    let key = key
        .trim_start_matches('/')
        .split('/')
        .map(|segment| {
            url::form_urlencoded::byte_serialize(segment.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        })
        .collect::<Vec<_>>()
        .join("/");
    if base.is_empty() {
        key
    } else {
        format!("{base}/{key}")
    }
}
