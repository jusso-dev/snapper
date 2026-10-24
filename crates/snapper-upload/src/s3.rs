//! S3-compatible uploads, including AWS S3 and Cloudflare R2.
//!
//! The secret access key is not stored on [`snapper_core::settings::UploadConfig`].
//! Callers load it from the keychain account `s3:<id>`.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use rusty_s3::actions::{PutObject, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use crate::{http_client, object_key, public_url, UploadError, UploadResult, UploadTarget};

pub struct S3Target {
    pub id: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub key_prefix: String,
    pub public_base_url: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

impl fmt::Debug for S3Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3Target")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("key_prefix", &self.key_prefix)
            .field("public_base_url", &self.public_base_url)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"***")
            .finish()
    }
}

impl S3Target {
    /// Presigned PUT URL. The secret is used to sign and is not placed in the path.
    pub fn presign(&self, name: &str) -> Result<(String, String), UploadError> {
        let endpoint = url::Url::parse(&self.endpoint)
            .map_err(|err| UploadError::Config(format!("S3 endpoint: {err}")))?;
        let bucket = Bucket::new(
            endpoint,
            UrlStyle::Path,
            self.bucket.clone(),
            self.region.clone(),
        )
        .map_err(|err| UploadError::Config(err.to_string()))?;
        let credentials = Credentials::new(&self.access_key_id, &self.secret_access_key);
        let key = object_key(&self.key_prefix, name);
        let action = PutObject::new(&bucket, Some(&credentials), &key);
        let signed = action.sign(Duration::from_secs(300));
        if signed.as_str().contains(&self.secret_access_key) {
            return Err(UploadError::Config(
                "the presigned URL contained the secret access key".to_owned(),
            ));
        }
        let public = if self.public_base_url.trim().is_empty() {
            let mut object = bucket
                .object_url(&key)
                .map_err(|err| UploadError::Config(err.to_string()))?;
            object.set_query(None);
            object.to_string()
        } else {
            public_url(&self.public_base_url, &key)
        };
        Ok((signed.to_string(), public))
    }
}

#[async_trait]
impl UploadTarget for S3Target {
    fn id(&self) -> &str {
        &self.id
    }

    async fn upload(
        &self,
        name: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<UploadResult, UploadError> {
        let (signed, public) = self.presign(name)?;
        let response = http_client()?
            .put(&signed)
            .header("content-type", mime)
            .body(bytes)
            .send()
            .await
            .map_err(|err| UploadError::Network(err.without_url().to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(UploadError::Rejected(format!("{status}: {body}")));
        }
        Ok(UploadResult {
            url: public,
            delete_url: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(endpoint: &str) -> S3Target {
        S3Target {
            id: "r2".to_owned(),
            endpoint: endpoint.to_owned(),
            bucket: "shots".to_owned(),
            region: "auto".to_owned(),
            key_prefix: "snap".to_owned(),
            public_base_url: "https://cdn.example".to_owned(),
            access_key_id: "AKIAEXAMPLE".to_owned(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
        }
    }

    #[test]
    fn presign_hides_the_secret_and_builds_a_public_url() {
        let target = target("https://example.r2.cloudflarestorage.com");
        let (signed, public) = target.presign("shot.png").expect("sign");
        assert!(signed.contains("X-Amz-Algorithm"), "{signed}");
        assert!(signed.contains("shots"), "{signed}");
        assert!(!signed.contains("wJalrXUtnFEMI"), "{signed}");
        assert_eq!(public, "https://cdn.example/snap/shot.png");
        let debug = format!("{target:?}");
        assert!(debug.contains("***"));
        assert!(!debug.contains("wJalrXUtnFEMI"));
    }

    #[tokio::test]
    async fn put_against_a_fake_s3_endpoint() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let target = target(&server.uri());
        let result = target
            .upload("shot.png", b"png".to_vec(), "image/png")
            .await
            .expect("put");
        assert_eq!(result.url, "https://cdn.example/snap/shot.png");
        assert!(result.delete_url.is_none());
    }

    #[tokio::test]
    #[ignore = "needs a local MinIO. Set MINIO_ENDPOINT, MINIO_ACCESS_KEY, MINIO_SECRET_KEY and MINIO_BUCKET"]
    async fn minio_put_returns_a_url() {
        let endpoint = std::env::var("MINIO_ENDPOINT").expect("MINIO_ENDPOINT");
        let access = std::env::var("MINIO_ACCESS_KEY").expect("MINIO_ACCESS_KEY");
        let secret = std::env::var("MINIO_SECRET_KEY").expect("MINIO_SECRET_KEY");
        let bucket = std::env::var("MINIO_BUCKET").expect("MINIO_BUCKET");
        let target = S3Target {
            id: "minio".to_owned(),
            endpoint,
            bucket,
            region: "us-east-1".to_owned(),
            key_prefix: "snapper-test".to_owned(),
            public_base_url: String::new(),
            access_key_id: access,
            secret_access_key: secret,
        };
        let result = target
            .upload("note.txt", b"hello".to_vec(), "text/plain")
            .await
            .expect("minio");
        assert!(!result.url.is_empty());
    }
}
