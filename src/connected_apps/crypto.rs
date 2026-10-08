//! Compatibility adapter for the shared authenticated encryption implementation.
use super::ConnectedAppError;

#[derive(Clone)]
pub struct CredentialCipher(crate::crypto::CredentialCipher);
impl CredentialCipher {
    pub fn from_hex_key(value: &str) -> Result<Self, ConnectedAppError> {
        crate::crypto::CredentialCipher::from_hex_key(value)
            .map(Self)
            .map_err(|_| ConnectedAppError::NotConfigured)
    }
    pub fn seal(&self, aad: &[u8], plaintext: &str) -> Result<Vec<u8>, ConnectedAppError> {
        self.0
            .seal(aad, plaintext)
            .map_err(|_| ConnectedAppError::Crypto)
    }
    pub fn open(&self, aad: &[u8], stored: &[u8]) -> Result<String, ConnectedAppError> {
        self.0
            .open(aad, stored)
            .map_err(|_| ConnectedAppError::Crypto)
    }
}
pub fn random_token() -> Result<String, ConnectedAppError> {
    crate::crypto::random_token().map_err(|_| ConnectedAppError::Crypto)
}
pub use crate::crypto::{pkce_challenge, sha256_hex};
