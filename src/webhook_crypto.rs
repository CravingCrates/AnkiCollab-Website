use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use std::fmt;

pub const WEBHOOK_KEY_BYTES: usize = 32;
pub const WEBHOOK_NONCE_BYTES: usize = 12;

fn zeroize_slice(buf: &mut [u8]) {
    for byte in buf {
        // SAFETY: `byte` is a valid mutable reference into `buf`.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// AES-256-GCM cipher for encrypting and decrypting deck webhook URLs at rest.
#[derive(Clone)]
pub struct WebhookCipher {
    key: [u8; WEBHOOK_KEY_BYTES],
}

impl Drop for WebhookCipher {
    fn drop(&mut self) {
        zeroize_slice(&mut self.key);
    }
}

impl fmt::Debug for WebhookCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookCipher(<redacted>)")
    }
}

impl WebhookCipher {
    /// Parses a 64-character hex-encoded 32-byte key.
    pub fn from_hex(hex_key: &str) -> Result<Self, String> {
        let trimmed = hex_key.trim();
        let mut bytes = hex::decode(trimmed)
            .map_err(|_| "WEBHOOK_ENCRYPTION_KEY must be valid hexadecimal".to_string())?;
        if bytes.len() != WEBHOOK_KEY_BYTES {
            let len = bytes.len();
            zeroize_slice(&mut bytes);
            return Err(format!(
                "WEBHOOK_ENCRYPTION_KEY must decode to {WEBHOOK_KEY_BYTES} bytes (got {len})"
            ));
        }
        let mut key = [0u8; WEBHOOK_KEY_BYTES];
        key.copy_from_slice(&bytes);
        zeroize_slice(&mut bytes);
        Ok(Self { key })
    }

    /// Loads `WEBHOOK_ENCRYPTION_KEY` from the environment.
    ///
    /// Returns `None` if unset or empty (disabling webhook configuration/delivery).
    /// Panics at startup if set to an invalid key so misconfiguration fails fast.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("WEBHOOK_ENCRYPTION_KEY") {
            Ok(raw) if !raw.trim().is_empty() => Some(
                Self::from_hex(&raw)
                    .unwrap_or_else(|err| panic!("Invalid WEBHOOK_ENCRYPTION_KEY: {err}")),
            ),
            _ => {
                tracing::warn!(
                    "WEBHOOK_ENCRYPTION_KEY is not set; deck Discord webhook notifications are disabled"
                );
                None
            }
        }
    }

    pub fn encrypt_url(
        &self,
        deck_id: i64,
        plaintext: &str,
    ) -> Result<(Vec<u8>, [u8; WEBHOOK_NONCE_BYTES]), String> {
        let mut nonce_bytes = [0u8; WEBHOOK_NONCE_BYTES];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|err| format!("Failed to generate random nonce: {err}"))?;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| "Failed to initialize AES-256-GCM cipher".to_string())?;
        let nonce = Nonce::from(nonce_bytes);
        let aad = deck_id.to_be_bytes();

        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| "Failed to encrypt webhook URL".to_string())?;

        Ok((ciphertext, nonce_bytes))
    }

    pub fn decrypt_url(
        &self,
        deck_id: i64,
        ciphertext: &[u8],
        nonce_bytes: &[u8],
    ) -> Result<String, String> {
        let nonce_arr: [u8; WEBHOOK_NONCE_BYTES] = nonce_bytes
            .try_into()
            .map_err(|_| "Invalid webhook nonce length".to_string())?;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| "Failed to initialize AES-256-GCM cipher".to_string())?;
        let nonce = Nonce::from(nonce_arr);
        let aad = deck_id.to_be_bytes();

        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| "Failed to decrypt webhook URL".to_string())?;

        String::from_utf8(plaintext).map_err(|err| {
            let mut bad = err.into_bytes();
            zeroize_slice(&mut bad);
            "Decrypted webhook URL is not valid UTF-8".to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn round_trip_encrypt_decrypt() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let url = "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let (ct, nonce) = cipher.encrypt_url(42, url).unwrap();
        assert_ne!(ct, url.as_bytes());
        let decrypted = cipher.decrypt_url(42, &ct, &nonce).unwrap();
        assert_eq!(decrypted, url);
    }

    #[test]
    fn wrong_deck_id_aad_fails() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let url = "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let (ct, nonce) = cipher.encrypt_url(42, url).unwrap();
        assert!(cipher.decrypt_url(43, &ct, &nonce).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let url = "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let (mut ct, nonce) = cipher.encrypt_url(42, url).unwrap();
        ct[0] ^= 0xFF;
        assert!(cipher.decrypt_url(42, &ct, &nonce).is_err());
    }

    #[test]
    fn invalid_key_lengths_rejected() {
        assert!(WebhookCipher::from_hex("abcd").is_err());
        assert!(WebhookCipher::from_hex(&"01".repeat(16)).is_err());
        assert!(WebhookCipher::from_hex(&"01".repeat(64)).is_err());
        assert!(WebhookCipher::from_hex("not-hex").is_err());
        assert!(WebhookCipher::from_hex(&format!("  {TEST_KEY_HEX}  ")).is_ok());
    }

    #[test]
    fn invalid_nonce_length_fails_cleanly() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let url = "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let (ct, _) = cipher.encrypt_url(42, url).unwrap();
        assert!(cipher.decrypt_url(42, &ct, &[0u8; 8]).is_err());
        assert!(cipher.decrypt_url(42, &ct, &[0u8; 16]).is_err());
    }

    #[test]
    fn distinct_nonces_per_encryption() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let url = "https://discord.com/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let (ct1, n1) = cipher.encrypt_url(1, url).unwrap();
        let (ct2, n2) = cipher.encrypt_url(1, url).unwrap();
        assert_ne!(n1, n2);
        assert_ne!(ct1, ct2);
    }

    #[test]
    fn debug_redacts_key() {
        let cipher = WebhookCipher::from_hex(TEST_KEY_HEX).unwrap();
        let dbg = format!("{cipher:?}");
        assert_eq!(dbg, "WebhookCipher(<redacted>)");
        assert!(!dbg.contains("012345"));
    }
}
