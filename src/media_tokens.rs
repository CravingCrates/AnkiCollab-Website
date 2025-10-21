use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

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
    pub fn new(
        secret: Vec<u8>,
        download_ttl: Duration,
    ) -> Result<Self, MediaTokenError> {
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
        let chrono_duration =
            ChronoDuration::from_std(duration).map_err(|_| MediaTokenError::InvalidTtl)?;
        Ok(Utc::now()
            .checked_add_signed(chrono_duration)
            .ok_or(MediaTokenError::InvalidTtl)?
            .timestamp())
    }

    fn ensure_not_expired(exp: i64) -> Result<(), MediaTokenError> {
        if Utc::now().timestamp() > exp {
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
            MediaTokenError::InvalidSecret => write!(f, "Media token secret must be at least 32 bytes"),
            MediaTokenError::InvalidTtl => write!(f, "Invalid token TTL"),
            MediaTokenError::InvalidFormat => write!(f, "Invalid token format"),
            MediaTokenError::InvalidSignature => write!(f, "Invalid token signature"),
            MediaTokenError::Expired => write!(f, "Token expired"),
            MediaTokenError::UnsupportedVersion(v) => write!(f, "Unsupported token version: {v}"),
            MediaTokenError::Decode(err) => write!(f, "Token decode error: {err}"),
            MediaTokenError::Serialization(err) => write!(f, "Token serialization error: {err}"),
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
