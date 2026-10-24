//! Update check.
//!
//! A signed `latest.json` is verified with minisign. Snapper never installs
//! an update on its own. The private key is not in this repository. Replace
//! [`BUNDLED_PUBLIC_KEY`] with the key from `minisign -G` before a real release.
//! See `docs/RELEASING.md`.

use serde::Deserialize;

/// Public key compiled into the app. This key has no matching private key in
/// the repository, so a release signature will not verify until it is replaced.
pub const BUNDLED_PUBLIC_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

/// Where a release publishes `latest.json`. Overridden with `SNAPPER_UPDATE_URL` for a fake server.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/jusso-dev/snapper/releases/latest/download/latest.json";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("the update manifest could not be read: {0}")]
    Fetch(String),
    #[error("the update signature was rejected")]
    BadSignature,
    #[error("the update manifest was not valid: {0}")]
    Manifest(String),
}

/// Verify `signature` over `manifest` and parse the JSON body.
pub fn verify_and_parse(
    manifest: &str,
    signature: &str,
    public_key: &str,
) -> Result<Release, UpdateError> {
    let key = minisign_verify::PublicKey::from_base64(public_key)
        .map_err(|_| UpdateError::BadSignature)?;
    let signature =
        minisign_verify::Signature::decode(signature).map_err(|_| UpdateError::BadSignature)?;
    key.verify(manifest.as_bytes(), &signature, false)
        .map_err(|_| UpdateError::BadSignature)?;
    serde_json::from_str(manifest).map_err(|err| UpdateError::Manifest(err.to_string()))
}

/// Download the manifest and its `.minisig` from `manifest_url`.
pub fn fetch(manifest_url: &str) -> Result<(String, String), UpdateError> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| UpdateError::Fetch(err.to_string()))?;
    let manifest = client
        .get(manifest_url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|err| UpdateError::Fetch(err.to_string()))?;
    let signature = client
        .get(format!("{manifest_url}.minisig"))
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|err| UpdateError::Fetch(err.to_string()))?;
    Ok((manifest, signature))
}

/// Check once. Failures are returned to the caller, which should stay quiet.
///
/// A newer verified release is announced. Nothing is downloaded or installed.
pub fn check_quiet() -> Result<(), UpdateError> {
    let url =
        std::env::var("SNAPPER_UPDATE_URL").unwrap_or_else(|_| DEFAULT_MANIFEST_URL.to_owned());
    let (manifest, signature) = fetch(&url)?;
    let release = verify_and_parse(&manifest, &signature, BUNDLED_PUBLIC_KEY)?;
    if is_newer(&release.version, snapper_core::version()) {
        crate::output::notify(
            "Snapper",
            &format!(
                "Snapper {} is available. Open the release page to install it yourself. Snapper will not install it for you.",
                release.version
            ),
        );
    }
    Ok(())
}

/// True when `candidate` is a later `major.minor.patch` than `current`.
///
/// A pre-release such as `1.2.0-beta.1` is older than `1.2.0`. A version that
/// does not parse is never newer, so a bad manifest cannot nag.
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(version: &str) -> Option<([u64; 3], bool)> {
        let version = version.trim().trim_start_matches('v');
        let version = version.split('+').next()?;
        let (core, pre) = match version.split_once('-') {
            Some((core, _)) => (core, true),
            None => (version, false),
        };
        let mut parts = [0u64; 3];
        let mut count = 0;
        for part in core.split('.') {
            *parts.get_mut(count)? = part.parse().ok()?;
            count += 1;
        }
        (count > 0).then_some((parts, pre))
    }
    match (parse(candidate), parse(current)) {
        (Some((new, new_pre)), Some((old, old_pre))) => {
            new > old || (new == old && old_pre && !new_pre)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_later_version_is_newer() {
        assert!(is_newer("0.0.2", "0.0.1"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("0.0.1", "0.0.2"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("latest", "1.0.0"));
    }

    fn sign(message: &str) -> (String, String) {
        let keypair = minisign::KeyPair::generate_unencrypted_keypair().expect("keypair");
        let signature = minisign::sign(
            Some(&keypair.pk),
            &keypair.sk,
            message.as_bytes(),
            None,
            None,
        )
        .expect("sign");
        let public = keypair.pk.to_base64();
        (public, signature.to_string())
    }

    #[test]
    fn a_valid_signature_parses_and_a_tampered_manifest_is_rejected() {
        let manifest = r#"{"version":"9.9.9","notes":"test","url":"https://example.com/snapper"}"#;
        let (public, signature) = sign(manifest);
        let release = verify_and_parse(manifest, &signature, &public).expect("verify");
        assert_eq!(release.version, "9.9.9");
        assert_eq!(release.notes, "test");

        let tampered = manifest.replace("9.9.9", "9.9.8");
        assert!(matches!(
            verify_and_parse(&tampered, &signature, &public),
            Err(UpdateError::BadSignature)
        ));
        assert!(verify_and_parse(manifest, "not a signature", &public).is_err());
    }

    #[test]
    fn fetch_reads_the_manifest_and_signature_from_a_fake_server() {
        // `fetch` uses a blocking client. Dropping that client inside the
        // server's runtime is not allowed, so the request runs on its own thread.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let (body, signature) = runtime.block_on(async {
            let server = wiremock::MockServer::start().await;
            let manifest = r#"{"version":"1.2.3","notes":"hello"}"#;
            wiremock::Mock::given(wiremock::matchers::path("/latest.json"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(manifest))
                .mount(&server)
                .await;
            wiremock::Mock::given(wiremock::matchers::path("/latest.json.minisig"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("sig"))
                .mount(&server)
                .await;
            let uri = format!("{}/latest.json", server.uri());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(fetch(&uri));
            });
            let fetched = rx.recv().expect("fetch thread").expect("fetch");
            drop(server);
            fetched
        });
        assert!(body.contains("1.2.3"));
        assert_eq!(signature, "sig");
    }
}
