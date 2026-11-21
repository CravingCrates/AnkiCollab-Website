use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use time::{OffsetDateTime, SignedDuration};

type HmacSha256 = Hmac<Sha256>;

const TOKEN_VERSION: u8 = 1;

#[derive(Clone)]
pub struct MediaTokenService {
    secret: Arc<Vec<u8>>,
    download_ttl: Duration,
}

impl std::fmt::Debug for MediaTokenService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaTokenService")
            .field("secret", &"<redacted>")
            .field("download_ttl", &self.download_ttl)
            .finish()
    }
}

impl MediaTokenService {
    pub fn new(secret: Vec<u8>, download_ttl: Duration) -> Result<Self, MediaTokenError> {
        if secret.len() < 32 {
            return Err(MediaTokenError::InvalidSecret);
        }

        Ok(Self {
            secret: Arc::new(secret),
            download_ttl,
        })
    }

    pub fn generate_download_token(
        &self,
        params: DownloadTokenParams,
    ) -> Result<String, MediaTokenError> {
        let exp = Self::expiry_from_duration(self.download_ttl)?;
        let claims = DownloadTokenClaims {
            hash: params.hash,
            user_id: params.user_id,
            deck_hash: params.deck_hash,
            filename: params.filename,
            exp,
        };

        self.encode(TokenPayload::Download(claims))
    }

    pub fn verify_download_token(
        &self,
        token: &str,
    ) -> Result<DownloadTokenClaims, MediaTokenError> {
        let envelope = self.decode(token)?;
        match envelope.payload {
            TokenPayload::Download(claims) => {
                Self::ensure_not_expired(claims.exp)?;
                Ok(claims)
            }
        }
    }

    fn encode(&self, payload: TokenPayload) -> Result<String, MediaTokenError> {
        let envelope = TokenEnvelope {
            version: TOKEN_VERSION,
            payload,
        };

        let payload_bytes =
            serde_json::to_vec(&envelope).map_err(MediaTokenError::Serialization)?;

        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| MediaTokenError::InvalidSecret)?;
        mac.update(&payload_bytes);
        let signature = mac.finalize().into_bytes();

        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature);

        Ok(format!("{payload_b64}.{signature_b64}"))
    }

    fn decode(&self, token: &str) -> Result<TokenEnvelope, MediaTokenError> {
        let mut parts = token.split('.');
        let payload_part = parts.next().ok_or(MediaTokenError::InvalidFormat)?;
        let signature_part = parts.next().ok_or(MediaTokenError::InvalidFormat)?;

        if parts.next().is_some() {
            return Err(MediaTokenError::InvalidFormat);
        }

        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload_part)
            .map_err(MediaTokenError::Decode)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature_part)
            .map_err(MediaTokenError::Decode)?;

        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| MediaTokenError::InvalidSecret)?;
        mac.update(&payload_bytes);
        mac.verify_slice(&signature)
            .map_err(|_| MediaTokenError::InvalidSignature)?;

        let envelope: TokenEnvelope =
            serde_json::from_slice(&payload_bytes).map_err(MediaTokenError::Serialization)?;

        if envelope.version != TOKEN_VERSION {
            return Err(MediaTokenError::UnsupportedVersion(envelope.version));
        }

        Ok(envelope)
    }

    fn expiry_from_duration(duration: Duration) -> Result<i64, MediaTokenError> {
        let ttl = SignedDuration::try_from(duration).map_err(|_| MediaTokenError::InvalidTtl)?;
        Ok(OffsetDateTime::now_utc()
            .checked_add(ttl)
            .ok_or(MediaTokenError::InvalidTtl)?
            .unix_timestamp())
    }

    fn ensure_not_expired(exp: i64) -> Result<(), MediaTokenError> {
        if OffsetDateTime::now_utc().unix_timestamp() > exp {
            return Err(MediaTokenError::Expired);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum MediaTokenError {
    InvalidSecret,
    InvalidTtl,
    InvalidFormat,
    InvalidSignature,
    Expired,
    UnsupportedVersion(u8),
    Decode(base64::DecodeError),
    Serialization(serde_json::Error),
}

impl fmt::Display for MediaTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSecret => {
                write!(f, "Media token secret must be at least 32 bytes")
            }
            Self::InvalidTtl => write!(f, "Invalid token TTL"),
            Self::InvalidFormat => write!(f, "Invalid token format"),
            Self::InvalidSignature => write!(f, "Invalid token signature"),
            Self::Expired => write!(f, "Token expired"),
            Self::UnsupportedVersion(v) => write!(f, "Unsupported token version: {v}"),
            Self::Decode(err) => write!(f, "Token decode error: {err}"),
            Self::Serialization(err) => write!(f, "Token serialization error: {err}"),
        }
    }
}

impl std::error::Error for MediaTokenError {}

#[derive(Debug, Clone)]
pub struct DownloadTokenParams {
    pub hash: String,
    pub user_id: i32,
    pub deck_hash: String,
    pub filename: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadTokenClaims {
    pub hash: String,
    pub user_id: i32,
    pub deck_hash: String,
    pub filename: Option<String>,
    pub exp: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TokenPayload {
    Download(DownloadTokenClaims),
}

#[derive(Serialize, Deserialize)]
struct TokenEnvelope {
    version: u8,
    payload: TokenPayload,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service(ttl: Duration) -> MediaTokenService {
        MediaTokenService::new(vec![0xAB; 32], ttl).expect("valid test secret")
    }

    #[test]
    fn download_token_round_trip_succeeds() {
        let service = test_service(Duration::from_secs(60));
        let token = service
            .generate_download_token(DownloadTokenParams {
                hash: "deck_hash_value".to_string(),
                user_id: 42,
                deck_hash: "parent_deck_hash".to_string(),
                filename: Some("image.png".to_string()),
            })
            .expect("token generation should succeed");

        let claims = service
            .verify_download_token(&token)
            .expect("fresh token should verify");
        assert_eq!(claims.hash, "deck_hash_value");
        assert_eq!(claims.user_id, 42);
        assert_eq!(claims.deck_hash, "parent_deck_hash");
        assert_eq!(claims.filename.as_deref(), Some("image.png"));
        assert!(claims.exp >= OffsetDateTime::now_utc().unix_timestamp());
    }

    #[test]
    fn expired_download_token_is_rejected() {
        let service = test_service(Duration::from_secs(60));
        let token = service
            .encode(TokenPayload::Download(DownloadTokenClaims {
                hash: "deck_hash_value".to_string(),
                user_id: 42,
                deck_hash: "parent_deck_hash".to_string(),
                filename: None,
                exp: OffsetDateTime::now_utc().unix_timestamp() - 10,
            }))
            .expect("encoding expired token should succeed");

        let err = service
            .verify_download_token(&token)
            .expect_err("expired token must fail verification");
        assert!(matches!(err, MediaTokenError::Expired));
    }

    #[test]
    fn overflowing_ttl_returns_invalid_ttl() {
        assert!(matches!(
            MediaTokenService::expiry_from_duration(Duration::MAX),
            Err(MediaTokenError::InvalidTtl)
        ));
        assert!(matches!(
            MediaTokenService::expiry_from_duration(Duration::from_secs(i64::MAX.cast_unsigned())),
            Err(MediaTokenError::InvalidTtl)
        ));
    }
}
