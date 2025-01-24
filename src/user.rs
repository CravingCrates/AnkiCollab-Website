use argon2::{
    password_hash::{
        rand_core::OsRng,
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString
    },
    Argon2
};

use async_trait::async_trait;
use axum::http::request::Parts;

use axum::extract::FromRequestParts;
use axum_extra::{
    extract::cookie::CookieJar,
};

use cookie::{
    Cookie as CookieBuilder,
    SameSite
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::{Duration, OffsetDateTime};
use tokio_postgres::Client;

use crate::error::AuthError;

const AUTH_COOKIE_NAME: &str = "__Host-ankicollabsession";
const COOKIE_MAX_AGE: i64 = 60 * 60 * 24 * 7; // 7 days in seconds

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub is_admin: bool,
}

impl User {
    pub fn id(&self) -> i32 { self.id }
    pub fn username(&self) -> String { self.username.clone() }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    sub: i32,          // user id
    exp: i64,          // expiration time
    iat: i64,          // issued at
}

#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub cookie: Option<String>
}
impl Clone for Credentials {
    fn clone(&self) -> Self {
        Self {
            username: self.username.clone(),
            password: self.password.clone(),
            cookie: self.cookie.clone(),
        }
    }
}

pub struct Auth {
    db: Arc<Client>,
    jwt_secret: String,
    cookie_secure: bool, // Should be true in production
}

impl Auth {
    pub fn new(
        db: Arc<Client>, 
        jwt_secret: String,
        cookie_secure: bool,
    ) -> Self {
        Self { 
            db, 
            jwt_secret,
            cookie_secure,
        }
    }

    pub async fn get_user_by_id(&self, user_id: i32) -> Result<User, AuthError> {
        let row = self
            .db
            .query_one(
                "SELECT id, username, is_admin
                 FROM users
                 WHERE id = $1",
                &[&user_id],
            )
            .await?;
        Ok(User {
            id: row.get(0),
            username: row.get(1),
            is_admin: row.get(2),
        })
    }
    
    pub async fn signup(&self, creds: Credentials) -> Result<User, AuthError> {
        // Normalize username to lowercase for case-insensitive comparison
        let normalized_username = creds.username.trim().to_lowercase();

        if normalized_username.is_empty() {
            return Err(AuthError::InvalidCredentials);
        }

        if !normalized_username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') || normalized_username.len() > 30 {
            return Err(AuthError::InvalidCredentials);
        }

        // Check if username already exists (case insensitive)
        let exists = self
            .db
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM users WHERE LOWER(username) = $1)",
                &[&normalized_username],
            )
            .await?
            .get::<_, bool>(0);

        if exists {
            return Err(AuthError::UsernameAlreadyExists);
        }

        // Validate password strength
        self.validate_password(&creds.password)?;

        // Hash password
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(creds.password.as_bytes(), &salt)
            .map_err(|e| AuthError::PasswordHash(e.to_string()))?
            .to_string();

        let row = self
            .db
            .query_one(
                "INSERT INTO users (username, password) 
                 VALUES ($1, $2) 
                 RETURNING id, username",
                &[&normalized_username, &password_hash],
            )
            .await?;

        Ok(User {
            id: row.get(0),
            username: row.get(1),
            is_admin: false,
        })
    }

    fn validate_password(&self, password: &str) -> Result<(), AuthError> {
        // Check password length
        if password.len() < 8 {
            return Err(AuthError::PasswordWeak);
        }

        // // Check password strength - we do that on the frontend so idc here
        // let password_regex = Regex::new(r"^(?=.*[a-z])(?=.*[A-Z])(?=.*\d).+$")
        //     .map_err(|_| AuthError::PasswordWeak)?;

        // if !password_regex.is_match(password) {
        //     return Err(AuthError::PasswordWeak);
        // }

        Ok(())
    }

    pub async fn login(&self, creds: Credentials) -> Result<String, AuthError> {
        let normalized_username = creds.username.to_lowercase();
        // Find user
        let row = self
            .db
            .query_opt(
                "SELECT id, password 
                 FROM users 
                 WHERE username = $1",
                &[&normalized_username]
            )
            .await?
            .ok_or(AuthError::InvalidCredentials)?;

        let user_id: i32 = row.get(0);
        let password_hash: String = row.get(1);

        // Verify password
        let parsed_hash = PasswordHash::new(&password_hash)
            .map_err(|e| AuthError::PasswordHash(e.to_string()))?;
        if argon2::Argon2::default()
            .verify_password(creds.password.as_bytes(), &parsed_hash)
            .is_err()
        {
            return Err(AuthError::InvalidCredentials);
        }

        // Generate JWT
        let now = OffsetDateTime::now_utc();
        let claims = Claims {
            sub: user_id,
            iat: now.unix_timestamp(),
            exp: (now + Duration::days(7)).unix_timestamp(),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        if creds.cookie.unwrap_or("".to_string()) == "on" {
            let cookie = CookieBuilder::build((AUTH_COOKIE_NAME, token))
            .path("/")
            .secure(self.cookie_secure)
            .http_only(true)
            .same_site(SameSite::Lax)
            .max_age(time::Duration::new(COOKIE_MAX_AGE, 0))
            .to_string();

            Ok(cookie)
        }
        else {
            let cookie = CookieBuilder::build((AUTH_COOKIE_NAME, token))
            .path("/")
            .secure(self.cookie_secure)
            .http_only(true)
            .same_site(SameSite::Lax)
            .to_string();

            Ok(cookie)
        }
    }

    pub async fn logout(&self) -> String {
        // Create expired cookie to clear the session
        CookieBuilder::build((AUTH_COOKIE_NAME, ""))
            .expires(time::OffsetDateTime::now_utc() - time::Duration::days(1))
            .path("/")
            .secure(self.cookie_secure)
            .http_only(true)
            .same_site(SameSite::Lax)
            .to_string()
    }

    pub fn verify_token(&self, token: &str) -> Result<i32, AuthError> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;

        Ok(token_data.claims.sub)
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for User
where
    S: Send + Sync,
{
    type Rejection = AuthError;
    
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract the cookie
        let cookies = CookieJar::from_request_parts(parts, state)
            .await
            .map_err(|_| AuthError::InternalError)?;
            
        let auth_cookie = cookies
            .get(AUTH_COOKIE_NAME)
            .ok_or(AuthError::NotAuthenticated)?;

        // Decode and verify the JWT
        let auth = parts
            .extensions
            .get::<Arc<Auth>>()
            .ok_or(AuthError::InternalError)?;
            
        let user_id = auth.verify_token(auth_cookie.value())
            .map_err(|_| AuthError::InvalidToken)?;

        // Retrieve user from database
        auth.get_user_by_id(user_id)
            .await
            .map_err(|_| AuthError::UserNotFound)
    }
}

pub fn require_auth(user: Option<User>) -> Result<User, AuthError> {
    user.ok_or(AuthError::Redirect("/login".to_string()))
}