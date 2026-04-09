use std::env::VarError;
use std::fmt::Debug;
use std::str::FromStr;
use tracing::{error, warn};

/// ServerConfig serves as an abstraction for the environment configuration. It is created once at startup.
#[derive(Debug)]
pub struct ServerConfig {
    pub database_url: String,
    pub sentry_url: String,
    pub s3_access_key_id: String,
    pub s3_secret_access_key: String,
    pub s3_domain: String,
    pub s3_media_bucket: String,
    pub jwt_secret: String,
    pub cookie_secure: bool,
    pub stats_cache_key: String,
    pub media_token_secret: String,
    pub media_proxy_url: String, // TODO: Discuss stronger typing with a Url type,
    pub port: u16,
    pub use_cloudflare_connecting_ip: bool,
}

impl ServerConfig {
    pub async fn new() -> Self {
        Self {
            database_url: env_or_default("DATABASE_URL", None),
            sentry_url: env_or_default("SENTRY_URL", None),
            s3_access_key_id: env_or_default("S3_ACCESS_KEY_ID", None),
            s3_secret_access_key: env_or_default("S3_SECRET_ACCESS_KEY", None),
            s3_domain: env_or_default("S3_DOMAIN", None),
            s3_media_bucket: env_or_default("S3_MEDIA_BUCKET", None),
            jwt_secret: env_or_default("JWT_SECRET", None),
            cookie_secure: env_or_default("COOKIE_SECURE", Some(true)),
            stats_cache_key: env_or_default("STATS_CACHE_KEY", None),
            media_token_secret: env_or_default("MEDIA_TOKEN_SECRET", None),
            media_proxy_url: env_or_default("MEDIA_PROXY_URL", Some("https://media.ankicollab.com".into())),
            port: env_or_default("PORT", Some(1337)),
            use_cloudflare_connecting_ip: env_or_default("USE_CLOUDFLARE_CONNECTING_IP", Some(true)),
        }
    }
}

/// Reads environment variable env_name and tries to parse it to the "Expected" type if it implements FromStr.
/// Optional default value. In case there is no environment variable env_name, the process will
/// end if no defaul is provided.
pub fn env_or_default<Expected>(env_name: &str, default: Option<Expected>) -> Expected
where
    Expected: FromStr + Debug,
{
    match std::env::var(env_name) {
        Ok(value) => match value.parse::<Expected>() {
            Ok(value) => value,
            Err(_) => match default {
                Some(default_value) => {
                    warn!("Environment variable {env_name} is set to a value that isn't in the right format. Falling back to default value {default_value:?}");
                    default_value
                }
                None => {
                    error!("Environment variable {env_name} is set to a value that isn't in the right format but required.");
                    std::process::exit(1);
                }
            },
        },
        Err(VarError::NotPresent) => match default {
            Some(default_value) => {
                warn!("Environment variable {env_name} is not set to a value. Defaulting to {default_value:?}.");
                default_value
            },
            None => {
                error!("Environment variable {env_name} is not set to a value but required.");
                std::process::exit(1);
            }
        },
        Err(e) => {
            error!("Failed to read environment variable {}: {}", env_name, e);
            default.unwrap()
        }
    }
}
