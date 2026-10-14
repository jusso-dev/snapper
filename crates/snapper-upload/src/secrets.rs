//! OS keychain access. Tests use [`MemorySecretStore`] and never call the keychain.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::UploadError;

pub trait SecretStore {
    fn get(&self, account: &str) -> Result<String, UploadError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), UploadError>;
}

/// `keyring::Entry::new("snapper", account)`.
pub struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<String, UploadError> {
        let entry = keyring::Entry::new("snapper", account)
            .map_err(|err| UploadError::Secret(err.to_string()))?;
        entry
            .get_password()
            .map_err(|err| UploadError::Secret(err.to_string()))
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), UploadError> {
        let entry = keyring::Entry::new("snapper", account)
            .map_err(|err| UploadError::Secret(err.to_string()))?;
        entry
            .set_password(secret)
            .map_err(|err| UploadError::Secret(err.to_string()))
    }
}

/// In-memory stand-in for tests.
#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, account: &str) -> Result<String, UploadError> {
        self.values
            .lock()
            .map_err(|_| UploadError::Secret("secret store lock was poisoned".to_owned()))?
            .get(account)
            .cloned()
            .ok_or_else(|| UploadError::Secret(format!("no secret stored for {account}")))
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), UploadError> {
        self.values
            .lock()
            .map_err(|_| UploadError::Secret("secret store lock was poisoned".to_owned()))?
            .insert(account.to_owned(), secret.to_owned());
        Ok(())
    }
}
