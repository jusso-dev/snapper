//! Custom HTTP multipart uploads.

use async_trait::async_trait;
use snapper_core::settings::HeaderSetting;

use crate::secrets::SecretStore;
use crate::{http_client, UploadError, UploadResult, UploadTarget};

pub struct HttpTarget<'a> {
    pub id: String,
    pub url: String,
    pub method: String,
    pub file_field: String,
    pub headers: Vec<HeaderSetting>,
    pub url_json_pointer: String,
    pub delete_url_json_pointer: String,
    pub secrets: &'a (dyn SecretStore + Sync),
}

#[async_trait]
impl UploadTarget for HttpTarget<'_> {
    fn id(&self) -> &str {
        &self.id
    }

    async fn upload(
        &self,
        name: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<UploadResult, UploadError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_owned())
            .mime_str(mime)
            .map_err(|err| UploadError::Config(err.to_string()))?;
        let form = reqwest::multipart::Form::new().part(self.file_field.clone(), part);
        let method = self
            .method
            .parse::<reqwest::Method>()
            .map_err(|err| UploadError::Config(err.to_string()))?;
        let mut request = http_client()?.request(method, &self.url).multipart(form);
        for header in &self.headers {
            let value = resolve_header(&self.id, &header.value, self.secrets)?;
            request = request.header(header.name.as_str(), value);
        }
        let response = request
            .send()
            .await
            .map_err(|err| UploadError::Network(err.without_url().to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| UploadError::Network(err.without_url().to_string()))?;
        if !status.is_success() {
            return Err(UploadError::Rejected(format!("{status}: {body}")));
        }
        let json = || {
            serde_json::from_str::<serde_json::Value>(&body)
                .map_err(|err| UploadError::Protocol(err.to_string()))
        };
        // An empty pointer means the response body is the URL itself.
        let url = if self.url_json_pointer.is_empty() {
            Some(body.trim().to_owned()).filter(|text| !text.is_empty())
        } else {
            pointer_string(&json()?, &self.url_json_pointer)?
        }
        .ok_or(UploadError::MissingUrl)?;
        let delete_url = if self.delete_url_json_pointer.is_empty() {
            None
        } else {
            pointer_string(&json()?, &self.delete_url_json_pointer)?
        };
        Ok(UploadResult { url, delete_url })
    }
}

/// A `keyring:` value may only name this target's own `http:{id}:` accounts,
/// so an imported config cannot send another target's secret to its host.
pub(crate) fn resolve_header(
    id: &str,
    value: &str,
    secrets: &(dyn SecretStore + Sync),
) -> Result<String, UploadError> {
    if let Some(account) = value.strip_prefix("keyring:") {
        if !account.starts_with(&format!("http:{id}:")) {
            return Err(UploadError::Config(format!(
                "header secret {account} does not belong to target {id}"
            )));
        }
        secrets.get(account)
    } else {
        Ok(value.to_owned())
    }
}

fn pointer_string(json: &serde_json::Value, pointer: &str) -> Result<Option<String>, UploadError> {
    if pointer.is_empty() {
        return Ok(None);
    }
    let pointer = if pointer.starts_with('/') {
        pointer.to_owned()
    } else {
        format!("/{pointer}")
    };
    Ok(json.pointer(&pointer).and_then(value_to_string))
}

fn value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySecretStore;
    use snapper_core::settings::HeaderSetting;
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn http_upload_reads_a_json_pointer_and_a_keychain_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("Authorization", "Bearer from-keychain"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "link": "https://cdn.example/a.png", "delete": "https://cdn.example/a.png/delete" }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let secrets = MemorySecretStore::default();
        secrets
            .set("http:custom:Authorization", "Bearer from-keychain")
            .expect("store");
        let target = HttpTarget {
            id: "custom".to_owned(),
            url: server.uri(),
            method: "POST".to_owned(),
            file_field: "file".to_owned(),
            headers: vec![HeaderSetting {
                name: "Authorization".to_owned(),
                value: "keyring:http:custom:Authorization".to_owned(),
            }],
            url_json_pointer: "/data/link".to_owned(),
            delete_url_json_pointer: "/data/delete".to_owned(),
            secrets: &secrets,
        };
        let result = target
            .upload("a.png", b"png".to_vec(), "image/png")
            .await
            .expect("upload");
        assert_eq!(result.url, "https://cdn.example/a.png");
        assert_eq!(
            result.delete_url.as_deref(),
            Some("https://cdn.example/a.png/delete")
        );
    }

    #[tokio::test]
    async fn network_errors_do_not_include_the_request_url() {
        let secrets = MemorySecretStore::default();
        let target = HttpTarget {
            id: "custom".to_owned(),
            url: "http://127.0.0.1:1/upload?key=query-secret".to_owned(),
            method: "POST".to_owned(),
            file_field: "file".to_owned(),
            headers: Vec::new(),
            url_json_pointer: String::new(),
            delete_url_json_pointer: String::new(),
            secrets: &secrets,
        };
        let err = target
            .upload("a.png", b"png".to_vec(), "image/png")
            .await
            .expect_err("connection refused");
        assert!(!err.to_string().contains("query-secret"), "{err}");
    }
}
