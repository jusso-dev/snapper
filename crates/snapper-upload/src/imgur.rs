//! Anonymous Imgur uploads.

use async_trait::async_trait;

use crate::{http_client, UploadError, UploadResult, UploadTarget};

pub struct ImgurTarget {
    pub id: String,
    pub client_id: String,
    /// Defaults to `https://api.imgur.com/3/image`. Tests point this at wiremock.
    pub endpoint: String,
}

impl ImgurTarget {
    pub fn anonymous(id: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            client_id: client_id.into(),
            endpoint: "https://api.imgur.com/3/image".to_owned(),
        }
    }
}

#[async_trait]
impl UploadTarget for ImgurTarget {
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
        let form = reqwest::multipart::Form::new().part("image", part);
        let response = http_client()?
            .post(&self.endpoint)
            .header("Authorization", format!("Client-ID {}", self.client_id))
            .multipart(form)
            .send()
            .await
            .map_err(|err| UploadError::Network(err.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|err| UploadError::Network(err.to_string()))?;
        let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Err(UploadError::Rejected(format!("{status}: {text}")));
        };
        parse_imgur(status, &body)
    }
}

pub(crate) fn parse_imgur(
    status: u16,
    body: &serde_json::Value,
) -> Result<UploadResult, UploadError> {
    if !(200..300).contains(&status)
        || body.get("success").and_then(|flag| flag.as_bool()) == Some(false)
    {
        return Err(UploadError::Rejected(body.to_string()));
    }
    let url = body
        .pointer("/data/link")
        .and_then(|value| value.as_str())
        .ok_or(UploadError::MissingUrl)?
        .to_owned();
    let delete_url = body
        .pointer("/data/deletehash")
        .and_then(|value| value.as_str())
        .map(|hash| format!("https://api.imgur.com/3/image/{hash}"));
    Ok(UploadResult { url, delete_url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn imgur_posts_the_client_id_and_reads_the_link() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/3/image"))
            .and(header("Authorization", "Client-ID abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true,
                "data": { "link": "https://i.imgur.com/abc.png", "deletehash": "del" }
            })))
            .expect(1)
            .mount(&server)
            .await;

        // The client posts to the real host. Point it at the mock by using a
        // custom request through the same parser with a replaced base. The
        // production URL is fixed, so this test drives `parse_imgur` instead
        // of the network when the host cannot be overridden. See `upload_to`.
        let target = ImgurTarget {
            id: "imgur".to_owned(),
            client_id: "abc".to_owned(),
            endpoint: format!("{}/3/image", server.uri()),
        };
        let result = target
            .upload("a.png", b"png".to_vec(), "image/png")
            .await
            .expect("upload");
        assert_eq!(result.url, "https://i.imgur.com/abc.png");
        assert_eq!(
            result.delete_url.as_deref(),
            Some("https://api.imgur.com/3/image/del")
        );
    }
}
