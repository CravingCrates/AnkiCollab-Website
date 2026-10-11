use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use axum_client_ip::ClientIp;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, CredentialID, DiscoverableAuthentication,
    DiscoverableKey, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, RequestChallengeResponse, Url, Webauthn, WebauthnBuilder,
    WebauthnError,
};
use webauthn_rs::DEFAULT_AUTHENTICATOR_TIMEOUT;

use crate::database::{client, AppState};
use crate::error::{AuthError, Error};
use crate::user::{Auth, SessionAge, User};

/// The most passkeys one account may hold.
pub const MAX_PASSKEYS_PER_USER: i64 = 5;

const MAX_PASSKEY_NAME_CHARS: usize = 64;
const DEFAULT_PASSKEY_NAME: &str = "Passkey";
const REGISTRATION_FAILED: &str = "passkey registration failed";

const MAX_USERNAME_BYTES: usize = 64;

pub const CHALLENGE_TTL: Duration =
    DEFAULT_AUTHENTICATOR_TIMEOUT.saturating_add(Duration::from_secs(30));

/// Cap on outstanding *anonymous* (sign-in) ceremonies. `POST /login/passkey/start`
pub const MAX_ANONYMOUS_CHALLENGES: usize = 10_000;

/// Cap on outstanding *registration* ceremonies. Kept separate from the anonymous pool
/// so that flooding the sign-in endpoint can never evict a signed-in user's
/// registration. A user also holds at most one at a time.
pub const MAX_REGISTRATION_CHALLENGES: usize = 1_000;

/// Outstanding anonymous ceremonies per client address (IPv6 grouped by /64). A flooder
/// evicts their own oldest entry, not anyone else's.
pub const MAX_ANONYMOUS_PER_IP: usize = 1_000;

const RATE_LIMIT_BURST: NonZeroU32 = NonZeroU32::new(150).unwrap();
const RATE_LIMIT_REFILL: Duration = Duration::from_secs(1);

pub const STEP_UP_WINDOW: Duration = Duration::from_mins(10);

#[derive(Debug)]
pub enum PasskeyError {
    Configuration(String),
    Ceremony(String),
    PossibleCompromise,
    BackupEligibilityInconsistent,
}

impl PasskeyError {
    fn from_webauthn(e: &WebauthnError) -> Self {
        match e {
            WebauthnError::CredentialPossibleCompromise => Self::PossibleCompromise,
            WebauthnError::CredentialBackupEligibilityInconsistent => {
                Self::BackupEligibilityInconsistent
            }
            other => Self::Ceremony(other.to_string()),
        }
    }
}

impl std::fmt::Display for PasskeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(msg) => write!(f, "passkey configuration error: {msg}"),
            Self::Ceremony(msg) => write!(f, "passkey ceremony error: {msg}"),
            Self::PossibleCompromise => write!(
                f,
                "signature counter did not advance (possible cloned credential)"
            ),
            Self::BackupEligibilityInconsistent => write!(
                f,
                "backup-eligible flag changed (retry by entering the username)"
            ),
        }
    }
}

impl std::error::Error for PasskeyError {}

#[derive(Debug)]
pub enum PasskeyHttpError {
    App(Error),
    RateLimited,
    LimitReached,
}

impl<E: Into<Error>> From<E> for PasskeyHttpError {
    fn from(e: E) -> Self {
        Self::App(e.into())
    }
}

impl IntoResponse for PasskeyHttpError {
    fn into_response(self) -> Response {
        match self {
            Self::App(e) => e.into_response(),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                // One refill period, so a client that waits this long is served again.
                [(header::RETRY_AFTER, "1")],
                Json(serde_json::json!({ "error": "rate_limited" })),
            )
                .into_response(),
            Self::LimitReached => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "passkey_limit",
                    "max": MAX_PASSKEYS_PER_USER,
                })),
            )
                .into_response(),
        }
    }
}

fn registration_failed(e: &PasskeyError) -> Error {
    tracing::warn!(error = %e, "passkey registration ceremony failed");
    Error::BadRequest(REGISTRATION_FAILED.to_string())
}

fn sign_in_failed(e: &PasskeyError, ip: IpAddr) -> Error {
    match e {
        PasskeyError::PossibleCompromise => tracing::warn!(
            %ip,
            "passkey sign-in refused: signature counter did not advance (possible cloned credential)"
        ),
        other => tracing::warn!(%ip, error = %other, "passkey sign-in ceremony failed"),
    }
    Error::Unauthorized
}

async fn require_password(
    auth: &Arc<Auth>,
    user_id: i32,
    password: &str,
) -> Result<(), PasskeyHttpError> {
    match auth.verify_password(user_id, password).await {
        Ok(()) => Ok(()),
        Err(AuthError::InvalidCredentials) => {
            tracing::warn!(user_id, "passkey change refused: wrong password");
            Err(Error::Unauthorized.into())
        }
        Err(other) => Err(other.into()),
    }
}

pub async fn no_store(response: Response) -> Response {
    let mut response = response;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

// ---------------------------------------------------------------------------
// Relying party configuration
// ---------------------------------------------------------------------------

fn validate_origin(origin: &Url) -> Result<(), PasskeyError> {
    let host = origin
        .host_str()
        .ok_or_else(|| PasskeyError::Configuration("RP_ORIGIN has no host".to_string()))?;

    let is_local = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    if origin.scheme() != "https" && !(is_local && origin.scheme() == "http") {
        return Err(PasskeyError::Configuration(
            "RP_ORIGIN must be https".to_string(),
        ));
    }
    if origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(PasskeyError::Configuration(
            "RP_ORIGIN must be a bare origin such as https://example.com".to_string(),
        ));
    }
    Ok(())
}

pub fn build_webauthn(
    rp_id: &str,
    rp_origin: &str,
    rp_name: &str,
) -> Result<Webauthn, PasskeyError> {
    let origin = Url::parse(rp_origin)
        .map_err(|e| PasskeyError::Configuration(format!("invalid RP_ORIGIN: {e}")))?;
    validate_origin(&origin)?;

    WebauthnBuilder::new(rp_id, &origin)
        .map_err(|e| PasskeyError::Configuration(format!("invalid RP_ID for origin: {e}")))?
        .rp_name(rp_name)
        .build()
        .map_err(|e| PasskeyError::Configuration(format!("webauthn build failed: {e}")))
}

fn required_env(name: &str, example: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => panic!("{name} must be set (for example {name}={example})"),
    }
}

#[must_use]
pub fn webauthn_from_env() -> (Webauthn, String) {
    let rp_id = required_env("RP_ID", "ankicollab.com");
    let rp_origin = required_env("RP_ORIGIN", "https://ankicollab.com");
    let rp_name = std::env::var("RP_NAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "AnkiCollab".to_string());

    let webauthn = build_webauthn(&rp_id, &rp_origin, &rp_name).unwrap_or_else(|e| {
        panic!("Invalid WebAuthn relying-party configuration ({rp_id} / {rp_origin}): {e}")
    });

    let site_origin = Url::parse(&rp_origin)
        .expect("RP_ORIGIN was accepted by build_webauthn")
        .origin()
        .ascii_serialization();

    tracing::info!(
        rp_id = %rp_id,
        rp_origin = %site_origin,
        "WebAuthn relying party configured"
    );
    (webauthn, site_origin)
}

fn ip_bucket(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
    }
}

pub struct PasskeyGuard {
    limiter: DefaultKeyedRateLimiter<IpAddr>,
}

impl std::fmt::Debug for PasskeyGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasskeyGuard").finish_non_exhaustive()
    }
}

impl Default for PasskeyGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyGuard {
    #[must_use]
    pub fn new() -> Self {
        let quota = Quota::with_period(RATE_LIMIT_REFILL)
            .expect("rate limit refill period is non-zero")
            .allow_burst(RATE_LIMIT_BURST);
        Self {
            limiter: RateLimiter::keyed(quota),
        }
    }

    pub fn check(&self, ip: IpAddr) -> Result<(), PasskeyHttpError> {
        self.limiter.check_key(&ip_bucket(ip)).map_err(|_| {
            tracing::debug!(%ip, "passkey endpoint rate limit hit");
            PasskeyHttpError::RateLimited
        })
    }

    pub fn sweep(&self) {
        self.limiter.retain_recent();
        self.limiter.shrink_to_fit();
    }
}

// ---------------------------------------------------------------------------
// Ceremony functions (no database access, so they are unit-testable)
// ---------------------------------------------------------------------------

pub fn begin_registration(
    webauthn: &Webauthn,
    user_uuid: Uuid,
    username: &str,
    exclude: Vec<CredentialID>,
) -> Result<(CreationChallengeResponse, PasskeyRegistration), PasskeyError> {
    // An empty exclude list goes out as absent rather than as `[]`. The protocol treats the
    // two as equivalent, so this is only about not putting noise in the payload - a
    // deliberate choice, and one the library's own tutorial does not make (it passes its
    // `Option` straight through, so a user with no existing keys still sends `[]`).
    let exclude = if exclude.is_empty() {
        None
    } else {
        Some(exclude)
    };

    // `start_passkey_registration` sends `residentKey: "discouraged"`, and this site keeps
    // that default, for the reason the module docs give: asking for a discoverable credential
    // (`preferred`) behaves as `required` in every major browser, and demanding resident
    // credentials can exhaust a CTAP2 authenticator's storage. What the library builds is
    // passed on exactly as it is - the options are never patched, and nothing outside the
    // prelude is reached for to do it.
    let (challenge, state) = webauthn
        .start_passkey_registration(user_uuid, username, username, exclude)
        .map_err(|e| PasskeyError::from_webauthn(&e))?;

    Ok((challenge, state))
}

pub fn finish_registration(
    webauthn: &Webauthn,
    credential: &RegisterPublicKeyCredential,
    state: &PasskeyRegistration,
) -> Result<Passkey, PasskeyError> {
    webauthn
        .finish_passkey_registration(credential, state)
        .map_err(|e| PasskeyError::from_webauthn(&e))
}

pub fn begin_authentication(
    webauthn: &Webauthn,
    passkeys: &[Passkey],
) -> Result<(RequestChallengeResponse, CeremonyState), PasskeyError> {
    let (response, state) = webauthn
        .start_passkey_authentication(passkeys)
        .map_err(|e| PasskeyError::from_webauthn(&e))?;

    Ok((response, CeremonyState::Authentication(Box::new(state))))
}

pub fn finish_authentication(
    webauthn: &Webauthn,
    credential: &PublicKeyCredential,
    state: &PasskeyAuthentication,
) -> Result<AuthenticationResult, PasskeyError> {
    webauthn
        .finish_passkey_authentication(credential, state)
        .map_err(|e| PasskeyError::from_webauthn(&e))
}

pub fn begin_discoverable_authentication(
    webauthn: &Webauthn,
) -> Result<(RequestChallengeResponse, Box<DiscoverableAuthentication>), PasskeyError> {
    let (response, state) = webauthn
        .start_discoverable_authentication()
        .map_err(|e| PasskeyError::from_webauthn(&e))?;
    Ok((response, Box::new(state)))
}

pub fn identify_discoverable_credential<'a>(
    webauthn: &Webauthn,
    credential: &'a PublicKeyCredential,
) -> Result<(Uuid, &'a [u8]), PasskeyError> {
    webauthn
        .identify_discoverable_authentication(credential)
        .map_err(|e| PasskeyError::from_webauthn(&e))
}

pub fn finish_discoverable_authentication(
    webauthn: &Webauthn,
    credential: &PublicKeyCredential,
    state: DiscoverableAuthentication,
    key: &DiscoverableKey,
) -> Result<AuthenticationResult, PasskeyError> {
    webauthn
        .finish_discoverable_authentication(credential, state, std::slice::from_ref(key))
        .map_err(|e| PasskeyError::from_webauthn(&e))
}

fn user_handle_matches(owner: Uuid, presented: Option<&[u8]>, required: bool) -> bool {
    match presented.filter(|handle| !handle.is_empty()) {
        None => !required,
        Some(handle) => Uuid::from_slice(handle).is_ok_and(|uuid| uuid == owner),
    }
}

fn is_allowed_name_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            ' ' | '-' | '_' | '.' | '\'' | '(' | ')'
                | '\u{2018}' // ‘ left single quotation mark
                | '\u{2019}' // ’ right single quotation mark (the iOS apostrophe)
                | '\u{201C}' // “ left double quotation mark
                | '\u{201D}' // ” right double quotation mark
        )
}

#[must_use]
pub fn sanitise_passkey_name(raw: Option<&str>) -> String {
    let collapsed = raw
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let name = collapsed
        .chars()
        .filter(|c| is_allowed_name_char(*c))
        .collect::<String>();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name
        .chars()
        .take(MAX_PASSKEY_NAME_CHARS)
        .collect::<String>();

    let name = name.trim();
    if name.is_empty() {
        DEFAULT_PASSKEY_NAME.to_string()
    } else {
        name.to_string()
    }
}

fn normalise_username(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
}

pub enum CeremonyState {
    Registration(Box<PasskeyRegistration>),
    Authentication(Box<PasskeyAuthentication>),
    Discoverable(Box<DiscoverableAuthentication>),
}

#[derive(Clone, Copy, Debug)]
pub enum Owner {
    Anonymous(IpAddr),
    User(i32),
}

struct Entry {
    user_id: Option<i32>,
    ip: Option<IpAddr>,
    state: CeremonyState,
    created_at: Instant,
    seq: u64,
}

#[derive(Clone, Copy)]
enum Pool {
    Anonymous,
    Registration,
}

#[derive(Default)]
struct StoreInner {
    entries: HashMap<Uuid, Entry>,
    anonymous: BTreeMap<u64, Uuid>,
    registrations: BTreeMap<u64, Uuid>,
    per_ip: HashMap<IpAddr, BTreeSet<u64>>,
    per_user: HashMap<i32, Uuid>,
    next_seq: u64,
}

impl StoreInner {
    fn pool_mut(&mut self, pool: Pool) -> &mut BTreeMap<u64, Uuid> {
        match pool {
            Pool::Anonymous => &mut self.anonymous,
            Pool::Registration => &mut self.registrations,
        }
    }

    fn pool_len(&self, pool: Pool) -> usize {
        match pool {
            Pool::Anonymous => self.anonymous.len(),
            Pool::Registration => self.registrations.len(),
        }
    }

    fn remove(&mut self, id: &Uuid) -> Option<Entry> {
        let entry = self.entries.remove(id)?;

        if entry.user_id.is_some() {
            self.registrations.remove(&entry.seq);
        } else {
            self.anonymous.remove(&entry.seq);
        }
        if let Some(ip) = entry.ip {
            if let Some(seqs) = self.per_ip.get_mut(&ip) {
                seqs.remove(&entry.seq);
                if seqs.is_empty() {
                    self.per_ip.remove(&ip);
                }
            }
        }
        if let Some(user_id) = entry.user_id {
            if self.per_user.get(&user_id) == Some(id) {
                self.per_user.remove(&user_id);
            }
        }
        Some(entry)
    }

    fn evict_oldest(&mut self, pool: Pool) -> bool {
        let oldest = self.pool_mut(pool).first_key_value().map(|(_, id)| *id);
        oldest.is_some_and(|id| self.remove(&id).is_some())
    }

    fn evict_oldest_for_ip(&mut self, ip: IpAddr) -> bool {
        let oldest = self
            .per_ip
            .get(&ip)
            .and_then(|seqs| seqs.first().copied())
            .and_then(|seq| self.anonymous.get(&seq).copied());
        oldest.is_some_and(|id| self.remove(&id).is_some())
    }

    fn purge_expired(&mut self) {
        for pool in [Pool::Anonymous, Pool::Registration] {
            while let Some((seq, id)) = self.pool_mut(pool).first_key_value().map(|(s, i)| (*s, *i))
            {
                match self.entries.get(&id) {
                    Some(entry) if entry.created_at.elapsed() < CHALLENGE_TTL => break,
                    Some(_) => {
                        self.remove(&id);
                    }
                    // An index entry with no ceremony behind it. Cannot happen through
                    // this module; dropped instead of looping forever if it ever does.
                    None => {
                        self.pool_mut(pool).remove(&seq);
                    }
                }
            }
        }
    }
}

pub struct ChallengeStore {
    inner: Mutex<StoreInner>,
}

// `AppState` derives `Debug`, and the ceremony state types are not guaranteed to
// implement it, so a manual impl is used here.
impl std::fmt::Debug for ChallengeStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChallengeStore")
            .field("len", &self.len())
            .finish()
    }
}

impl Default for ChallengeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChallengeStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(StoreInner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, StoreInner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::error!(
                    "passkey challenge store was poisoned by a panic; discarding all in-flight ceremonies"
                );
                let mut guard = poisoned.into_inner();
                *guard = StoreInner::default();
                self.inner.clear_poison();
                guard
            }
        }
    }

    pub fn insert(&self, state: CeremonyState, owner: Owner) -> Uuid {
        self.insert_with_created_at(state, owner, Instant::now())
    }

    /// Exists so expiry is testable without sleeping.
    pub fn insert_with_created_at(
        &self,
        state: CeremonyState,
        owner: Owner,
        created_at: Instant,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let mut inner = self.lock();
        inner.purge_expired();

        let (user_id, ip) = match owner {
            Owner::User(user_id) => (Some(user_id), None),
            Owner::Anonymous(ip) => (None, Some(ip_bucket(ip))),
        };

        if let Some(user_id) = user_id {
            // A new registration attempt replaces the user's previous one.
            if let Some(previous) = inner.per_user.get(&user_id).copied() {
                inner.remove(&previous);
            }
            while inner.pool_len(Pool::Registration) >= MAX_REGISTRATION_CHALLENGES
                && inner.evict_oldest(Pool::Registration)
            {}
        }
        if let Some(ip) = ip {
            while inner
                .per_ip
                .get(&ip)
                .is_some_and(|seqs| seqs.len() >= MAX_ANONYMOUS_PER_IP)
                && inner.evict_oldest_for_ip(ip)
            {}
            while inner.pool_len(Pool::Anonymous) >= MAX_ANONYMOUS_CHALLENGES
                && inner.evict_oldest(Pool::Anonymous)
            {}
        }

        let seq = inner.next_seq;
        inner.next_seq += 1;
        match user_id {
            Some(user_id) => {
                inner.registrations.insert(seq, id);
                inner.per_user.insert(user_id, id);
            }
            None => {
                inner.anonymous.insert(seq, id);
            }
        }
        if let Some(ip) = ip {
            inner.per_ip.entry(ip).or_default().insert(seq);
        }
        inner.entries.insert(
            id,
            Entry {
                user_id,
                ip,
                state,
                created_at,
                seq,
            },
        );
        id
    }

    pub fn take(&self, id: &Uuid, expect_user_id: Option<i32>) -> Option<CeremonyState> {
        let entry = self.lock().remove(id)?;

        if entry.created_at.elapsed() >= CHALLENGE_TTL {
            return None;
        }
        if entry.user_id != expect_user_id {
            return None;
        }
        Some(entry.state)
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Outstanding sign-in ceremonies. Test-only: nothing on the running site needs it.
    #[cfg(test)]
    pub fn len_anonymous(&self) -> usize {
        self.lock().anonymous.len()
    }

    /// Outstanding registration ceremonies. Test-only.
    #[cfg(test)]
    pub fn len_registrations(&self) -> usize {
        self.lock().registrations.len()
    }

    /// Drops every expired ceremony.
    pub fn retain_recent(&self) {
        self.lock().purge_expired();
    }
}

// ---------------------------------------------------------------------------
// Request and response bodies
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct RegisterStartResponse {
    pub challenge_id: Uuid,
    #[serde(flatten)]
    pub public_key: CreationChallengeResponse,
}

#[derive(Deserialize)]
pub struct RegisterFinishRequest {
    pub challenge_id: Uuid,
    #[serde(default)]
    pub name: Option<String>,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Deserialize)]
pub struct RegisterStartRequest {
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct DeletePasskeyRequest {
    pub passkey_id: i32,
    pub password: String,
}

#[derive(Deserialize)]
pub struct LoginStartRequest {
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Serialize)]
pub struct LoginStartResponse {
    pub challenge_id: Uuid,
    #[serde(flatten)]
    pub public_key: RequestChallengeResponse,
}

#[derive(Deserialize)]
pub struct LoginFinishRequest {
    pub challenge_id: Uuid,
    #[serde(default)]
    pub persistent: bool,
    pub credential: PublicKeyCredential,
}

#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct PasskeyListItem {
    pub id: i32,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

fn parse_stored_passkey(raw: serde_json::Value, context: &str) -> Option<Passkey> {
    match serde_json::from_value(raw) {
        Ok(passkey) => Some(passkey),
        Err(e) => {
            tracing::error!(error = %e, context, "stored passkey is unreadable");
            None
        }
    }
}

async fn existing_credential_ids(
    db_state: &AppState,
    user_id: i32,
) -> Result<Vec<CredentialID>, Error> {
    let conn = client(db_state).await?;
    let rows = conn
        .query(
            "SELECT credential_id FROM public.passkey_credentials WHERE user_id = $1",
            &[&user_id],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| CredentialID::from(row.get::<_, Vec<u8>>(0)))
        .collect())
}

async fn user_uuid(db_state: &AppState, user_id: i32) -> Result<Uuid, Error> {
    let conn = client(db_state).await?;
    let row = conn
        .query_one(
            "SELECT uuid FROM public.users WHERE id = $1 AND deleted_at IS NULL",
            &[&user_id],
        )
        .await?;
    Ok(row.get(0))
}

async fn passkeys_by_username(db_state: &AppState, username: &str) -> Result<Vec<Passkey>, Error> {
    if username.len() > MAX_USERNAME_BYTES {
        return Ok(Vec::new());
    }
    let conn = client(db_state).await?;
    let rows = conn
        .query(
            "SELECT pc.passkey
             FROM public.users u
             LEFT JOIN public.passkey_credentials pc ON pc.user_id = u.id
             WHERE u.username = $1 AND u.deleted_at IS NULL",
            &[&username],
        )
        .await?;

    Ok(rows
        .iter()
        .filter_map(|row| row.get::<_, Option<serde_json::Value>>(0))
        .filter_map(|raw| parse_stored_passkey(raw, "username lookup"))
        .collect())
}

async fn passkey_for_discoverable(
    db_state: &AppState,
    credential_id: &[u8],
    handle: Uuid,
) -> Result<Option<Passkey>, Error> {
    let conn = client(db_state).await?;
    let row = conn
        .query_opt(
            "SELECT pc.passkey
             FROM public.passkey_credentials pc
             JOIN public.users u ON u.id = pc.user_id
             WHERE pc.credential_id = $1 AND u.uuid = $2 AND u.deleted_at IS NULL",
            &[&credential_id, &handle],
        )
        .await?;

    Ok(row.and_then(|row| parse_stored_passkey(row.get(0), "discoverable lookup")))
}

pub async fn passkeys_for_profile(
    db_state: &AppState,
    user_id: i32,
) -> Result<Vec<PasskeyListItem>, Error> {
    let conn = client(db_state).await?;
    let rows = conn
        .query(
            "SELECT id, name,
                    to_char(created_at, 'YYYY-MM-DD') AS created_at,
                    to_char(last_used_at, 'YYYY-MM-DD') AS last_used_at
             FROM public.passkey_credentials
             WHERE user_id = $1
             ORDER BY created_at DESC",
            &[&user_id],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|row| PasskeyListItem {
            id: row.get(0),
            name: row.get(1),
            created_at: row.get(2),
            last_used_at: row.get(3),
        })
        .collect())
}

pub async fn passkey_count(db_state: &AppState, user_id: i32) -> Result<i64, Error> {
    let conn = client(db_state).await?;
    let row = conn
        .query_one(
            "SELECT count(*) FROM public.passkey_credentials WHERE user_id = $1",
            &[&user_id],
        )
        .await?;
    Ok(row.get(0))
}

fn is_unique_violation(e: &tokio_postgres::Error) -> bool {
    e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
}

pub async fn post_register_start(
    State(db_state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<Auth>>,
    session: SessionAge,
    user: User,
    Json(request): Json<RegisterStartRequest>,
) -> Result<Json<RegisterStartResponse>, PasskeyHttpError> {
    match request
        .password
        .as_deref()
        .filter(|password| !password.is_empty())
    {
        Some(password) => require_password(&auth, user.id(), password).await?,
        None if session.is_within(STEP_UP_WINDOW) => {}
        None => {
            tracing::warn!(
                user_id = user.id(),
                "passkey registration refused: session is too old to skip the password"
            );
            return Err(Error::Unauthorized.into());
        }
    }

    if passkey_count(&db_state, user.id()).await? >= MAX_PASSKEYS_PER_USER {
        return Err(PasskeyHttpError::LimitReached);
    }

    let exclude = existing_credential_ids(&db_state, user.id()).await?;
    let uuid = user_uuid(&db_state, user.id()).await?;

    let (public_key, state) = begin_registration(
        &db_state.webauthn,
        uuid,
        &user.username(),
        exclude,
    )
    .map_err(|e| {
        // Nothing the caller sent can make this fail, so it is a server fault.
        tracing::error!(error = %e, user_id = user.id(), "passkey registration could not start");
        Error::Unknown
    })?;

    let challenge_id = db_state.passkey_challenges.insert(
        CeremonyState::Registration(Box::new(state)),
        Owner::User(user.id()),
    );

    Ok(Json(RegisterStartResponse {
        challenge_id,
        public_key,
    }))
}

pub async fn post_register_finish(
    State(db_state): State<Arc<AppState>>,
    user: User,
    Json(request): Json<RegisterFinishRequest>,
) -> Result<Json<OkResponse>, PasskeyHttpError> {
    let fail = || Error::BadRequest(REGISTRATION_FAILED.to_string());

    let name = sanitise_passkey_name(request.name.as_deref());

    let state = db_state
        .passkey_challenges
        .take(&request.challenge_id, Some(user.id()))
        .ok_or_else(fail)?;

    let CeremonyState::Registration(registration) = state else {
        return Err(fail().into());
    };

    let passkey = finish_registration(
        &db_state.webauthn,
        &request.credential,
        registration.as_ref(),
    )
    .map_err(|e| registration_failed(&e))?;

    let credential_id = passkey.cred_id().clone();
    let credential_bytes: &[u8] = credential_id.as_ref();
    let serialized = serde_json::to_value(&passkey).map_err(|e| {
        tracing::error!(error = %e, "passkey serialization failed");
        Error::Unknown
    })?;

    let mut conn = client(&db_state).await?;
    let tx = conn.transaction().await?;

    let locked = tx
        .query_opt(
            "SELECT 1 FROM public.users WHERE id = $1 AND deleted_at IS NULL FOR NO KEY UPDATE",
            &[&user.id()],
        )
        .await?;
    if locked.is_none() {
        return Err(Error::Unauthorized.into());
    }

    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM public.passkey_credentials WHERE user_id = $1",
            &[&user.id()],
        )
        .await?
        .get(0);
    if count >= MAX_PASSKEYS_PER_USER {
        return Err(PasskeyHttpError::LimitReached);
    }

    let inserted = tx
        .execute(
            "INSERT INTO public.passkey_credentials (user_id, name, credential_id, passkey)
             VALUES ($1, $2, $3, $4)",
            &[&user.id(), &name, &credential_bytes, &serialized],
        )
        .await;
    match inserted {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            tracing::warn!(
                user_id = user.id(),
                "refusing a passkey whose credential id is already registered"
            );
            return Err(fail().into());
        }
        Err(e) => return Err(e.into()),
    }
    tx.commit().await?;

    Ok(Json(OkResponse { ok: true }))
}

pub async fn post_login_start(
    ClientIp(ip): ClientIp,
    State(db_state): State<Arc<AppState>>,
    Json(request): Json<LoginStartRequest>,
) -> Result<Json<LoginStartResponse>, PasskeyHttpError> {
    db_state.passkey_guard.check(ip)?;

    let (public_key, state) = if let Some(username) =
        normalise_username(request.username.as_deref())
    {
        let passkeys = passkeys_by_username(&db_state, &username).await?;
        if passkeys.is_empty() {
            return Err(Error::Unauthorized.into());
        }
        begin_authentication(&db_state.webauthn, &passkeys).map_err(|e| sign_in_failed(&e, ip))?
    } else {
        let (response, state) = begin_discoverable_authentication(&db_state.webauthn)
            .map_err(|e| sign_in_failed(&e, ip))?;
        (response, CeremonyState::Discoverable(state))
    };

    let challenge_id = db_state
        .passkey_challenges
        .insert(state, Owner::Anonymous(ip));

    Ok(Json(LoginStartResponse {
        challenge_id,
        public_key,
    }))
}

pub async fn post_login_finish(
    ClientIp(ip): ClientIp,
    Extension(auth): Extension<Arc<Auth>>,
    State(db_state): State<Arc<AppState>>,
    Json(request): Json<LoginFinishRequest>,
) -> Result<impl IntoResponse, PasskeyHttpError> {
    db_state.passkey_guard.check(ip)?;

    let credential = &request.credential;
    let state = db_state
        .passkey_challenges
        .take(&request.challenge_id, None)
        .ok_or(Error::Unauthorized)?;

    let discoverable = matches!(state, CeremonyState::Discoverable(_));

    let result = match state {
        CeremonyState::Authentication(authentication) => {
            finish_authentication(&db_state.webauthn, credential, &authentication)
                .map_err(|e| sign_in_failed(&e, ip))?
        }
        CeremonyState::Discoverable(ceremony) => {
            let (handle, credential_id) =
                identify_discoverable_credential(&db_state.webauthn, credential)
                    .map_err(|e| sign_in_failed(&e, ip))?;

            let stored = passkey_for_discoverable(&db_state, credential_id, handle)
                .await?
                .ok_or(Error::Unauthorized)?;

            finish_discoverable_authentication(
                &db_state.webauthn,
                credential,
                *ceremony,
                &DiscoverableKey::from(&stored),
            )
            .map_err(|e| sign_in_failed(&e, ip))?
        }
        CeremonyState::Registration(_) => return Err(Error::Unauthorized.into()),
    };

    let credential_id = result.cred_id().clone();
    let credential_bytes: &[u8] = credential_id.as_ref();

    let mut conn = client(&db_state).await?;
    let tx = conn.transaction().await?;

    let row = tx
        .query_opt(
            "SELECT pc.user_id, u.uuid, pc.passkey
             FROM public.passkey_credentials pc
             JOIN public.users u ON u.id = pc.user_id
             WHERE pc.credential_id = $1 AND u.deleted_at IS NULL
             FOR UPDATE OF pc",
            &[&credential_bytes],
        )
        .await?
        // Also the outcome when the passkey was removed after the ceremony started:
        // a scoped ceremony carries the credentials as they were at `start`.
        .ok_or(Error::Unauthorized)?;

    let user_id: i32 = row.get(0);
    let owner: Uuid = row.get(1);

    if !user_handle_matches(owner, credential.get_user_unique_id(), discoverable) {
        tracing::warn!(%ip, user_id, "passkey sign-in refused: user handle does not match the credential's owner");
        return Err(Error::Unauthorized.into());
    }

    let mut updated: Passkey = parse_stored_passkey(row.get(2), "sign-in").ok_or(Error::Unknown)?;

    if updated.update_credential(&result).is_none() {
        tracing::error!(
            user_id,
            "verified assertion does not match the stored credential id"
        );
        return Err(Error::Unauthorized.into());
    }
    let serialized = serde_json::to_value(&updated).map_err(|e| {
        tracing::error!(error = %e, "passkey serialization failed during sign-in");
        Error::Unknown
    })?;

    tx.execute(
        "UPDATE public.passkey_credentials
         SET passkey = $1, last_used_at = now()
         WHERE credential_id = $2",
        &[&serialized, &credential_bytes],
    )
    .await?;
    tx.commit().await?;

    let cookie = auth.complete_login(user_id, ip, request.persistent).await?;

    let mut response = Json(OkResponse { ok: true }).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|e| {
            tracing::error!(error = %e, "session cookie was not a valid header value");
            Error::Unknown
        })?,
    );
    Ok(response)
}

pub async fn post_delete_passkey(
    State(db_state): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<Auth>>,
    user: User,
    Json(request): Json<DeletePasskeyRequest>,
) -> Result<Json<OkResponse>, PasskeyHttpError> {
    require_password(&auth, user.id(), &request.password).await?;

    let conn = client(&db_state).await?;
    let deleted = conn
        .execute(
            "DELETE FROM public.passkey_credentials WHERE id = $1 AND user_id = $2",
            &[&request.passkey_id, &user.id()],
        )
        .await?;

    if deleted == 0 {
        tracing::warn!(
            user_id = user.id(),
            passkey_id = request.passkey_id,
            "passkey delete matched no row"
        );
    }

    Ok(Json(OkResponse { ok: true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn test_webauthn() -> Webauthn {
        build_webauthn("localhost", "http://localhost:1337", "AnkiCollab")
            .expect("test webauthn must build")
    }

    fn test_guard() -> PasskeyGuard {
        PasskeyGuard::new()
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    /// Spreads `i` over distinct /24s so tests do not collide with the per-address cap.
    fn spread_addr(i: usize) -> IpAddr {
        let octet = |shift: u32| u8::try_from((i >> shift) & 0xFF).unwrap_or(0);
        IpAddr::V4(Ipv4Addr::new(10, octet(16), octet(8), octet(0)))
    }

    fn dummy_registration_state() -> CeremonyState {
        let webauthn = test_webauthn();
        let (_ccr, state) =
            begin_registration(&webauthn, Uuid::new_v4(), "alice", Vec::new()).unwrap();
        CeremonyState::Registration(Box::new(state))
    }

    /// A stored passkey, built the way the database hands one back: from JSON. The
    /// values come from the `webauthn-rs` test suite.
    fn stored_passkey(credential_id_byte: u8) -> Passkey {
        let mut id = vec![credential_id_byte; 64];
        id[0] = credential_id_byte;
        let json = serde_json::json!({
            "cred": {
                "cred_id": base64url(&id),
                "cred": {
                    "type_": "ES256",
                    "key": { "EC_EC2": {
                        "curve": "SECP256R1",
                        "x": [194,126,127,109,252,23,131,21,252,6,223,99,44,254,140,27,230,17,94,5,133,28,104,41,144,69,171,149,161,26,200,243],
                        "y": [143,123,183,156,24,178,21,248,117,159,162,69,171,52,188,252,26,59,6,47,103,92,19,58,117,103,249,0,219,8,95,196]
                    }}
                },
                "counter": 2,
                "transports": ["usb", "nfc"],
                "user_verified": true,
                "backup_eligible": false,
                "backup_state": false,
                "registration_policy": "required",
                "extensions": { "cred_protect": "NotRequested", "hmac_create_secret": "NotRequested" },
                "attestation": { "data": "None", "metadata": "None" },
                "attestation_format": "None"
            }
        });
        serde_json::from_value(json).expect("a stored passkey must deserialise")
    }

    fn base64url(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            let chars = chunk.len() + 1;
            for i in 0..chars {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }

    // -- relying party -------------------------------------------------------

    #[test]
    fn rp_id_must_be_an_effective_domain_of_the_origin() {
        assert!(build_webauthn("ankicollab.com", "https://ankicollab.com", "AnkiCollab").is_ok());
        assert!(
            build_webauthn("evil.example", "https://ankicollab.com", "AnkiCollab").is_err(),
            "an rp_id that is not a suffix of the origin must be rejected"
        );
    }

    #[test]
    fn origin_must_be_https_and_bare() {
        for bad in [
            "http://ankicollab.com",
            "https://ankicollab.com/login",
            "https://ankicollab.com/?x=1",
            "https://user:pw@ankicollab.com",
            "ankicollab.com",
        ] {
            assert!(
                build_webauthn("ankicollab.com", bad, "AnkiCollab").is_err(),
                "{bad} must be refused"
            );
        }
        // A trailing slash is the same origin and must keep working.
        assert!(build_webauthn("ankicollab.com", "https://ankicollab.com/", "AnkiCollab").is_ok());
    }

    #[test]
    fn development_pair_is_valid() {
        assert!(build_webauthn("localhost", "http://localhost:1337", "AnkiCollab").is_ok());
    }

    // -- registration options ------------------------------------------------

    #[test]
    fn registration_keeps_the_library_default_of_discouraged_resident_keys() {
        let (challenge, _state) =
            begin_registration(&test_webauthn(), Uuid::new_v4(), "alice", Vec::new())
                .expect("registration must start");

        let json = serde_json::to_value(challenge).expect("the challenge must serialise");
        assert_eq!(
            json["publicKey"]["authenticatorSelection"]["residentKey"].as_str(),
            Some("discouraged"),
            "got: {json}"
        );
    }

    #[test]
    fn registration_challenge_puts_the_options_at_the_top_level() {
        let (ccr, _state) =
            begin_registration(&test_webauthn(), Uuid::new_v4(), "alice", Vec::new())
                .expect("registration must start");

        let json = serde_json::to_value(RegisterStartResponse {
            challenge_id: Uuid::new_v4(),
            public_key: ccr,
        })
        .expect("the response must serialise");

        assert!(json.get("challenge_id").is_some(), "got: {json}");
        let options = json
            .get("publicKey")
            .unwrap_or_else(|| panic!("'publicKey' must be at the top level, got: {json}"));
        assert!(
            options
                .get("challenge")
                .is_some_and(serde_json::Value::is_string),
            "the challenge must be a base64url string, got: {options}"
        );
        assert!(
            options
                .get("user")
                .and_then(|user| user.get("id"))
                .is_some_and(serde_json::Value::is_string),
            "the user handle must be a base64url string, got: {options}"
        );
        assert!(
            options.get("excludeCredentials").is_none(),
            "an empty exclude list must be omitted, got: {options}"
        );
    }

    // -- sign-in options -----------------------------------------------------

    #[test]
    fn a_missing_or_null_username_means_the_discoverable_flow() {
        // The browser sends `{"username": null}` while the box is empty and a string
        // once it is filled in.
        let absent: LoginStartRequest =
            serde_json::from_str("{}").expect("an absent field must be accepted");
        assert!(absent.username.is_none());

        let null: LoginStartRequest = serde_json::from_str(r#"{"username":null}"#)
            .expect("an explicit null must be accepted");
        assert!(null.username.is_none());

        let named: LoginStartRequest =
            serde_json::from_str(r#"{"username":"alice"}"#).expect("a username must be accepted");
        assert_eq!(named.username.as_deref(), Some("alice"));
    }

    #[test]
    fn usernames_are_normalised_like_password_login() {
        assert_eq!(normalise_username(None), None);
        assert_eq!(normalise_username(Some("")), None);
        assert_eq!(normalise_username(Some("   ")), None);
        assert_eq!(
            normalise_username(Some("  Alice ")).as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn discoverable_start_is_passed_through_untouched() {
        let (response, _state) = begin_discoverable_authentication(&test_webauthn())
            .expect("discoverable authentication must start");

        // An empty list is what makes the browser show its own passkey picker
        // instead of requiring the site to name the account up front.
        assert!(response.public_key.allow_credentials.is_empty());

        let json = serde_json::to_value(LoginStartResponse {
            challenge_id: Uuid::new_v4(),
            public_key: response,
        })
        .expect("the response must serialise");

        // The library's `mediation` value is handed on exactly as produced, not cleared:
        // the struct is documented as opaque, and the field sits beside `publicKey` in the
        // envelope, which is not where `passkeys.js` looks. Mediation is decided client-side.
        assert_eq!(
            json.get("mediation").and_then(serde_json::Value::as_str),
            Some("conditional"),
            "the library's mediation value must be passed through, got: {json}"
        );

        let options = json
            .get("publicKey")
            .unwrap_or_else(|| panic!("'publicKey' must be at the top level, got: {json}"));
        assert_eq!(
            options
                .get("allowCredentials")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(0),
            "got: {options}"
        );
        assert!(
            options
                .get("userVerification")
                .is_some_and(serde_json::Value::is_string),
            "the ceremony must ask for user verification, got: {options}"
        );
    }

    #[test]
    fn real_credentials_keep_their_transports() {
        // The credentials go out exactly as the library builds them, transports
        // included, so the browser can pick the right authenticator without probing.
        let (response, _) = begin_authentication(&test_webauthn(), &[stored_passkey(7)]).unwrap();
        assert_eq!(response.public_key.allow_credentials.len(), 1);
        assert_eq!(
            response.public_key.allow_credentials[0]
                .transports
                .as_ref()
                .map(Vec::len),
            Some(2),
            "the stored credential lists usb and nfc"
        );
    }

    // -- user handle ---------------------------------------------------------

    #[test]
    fn user_handle_must_name_the_credentials_owner() {
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();

        // Discoverable: the handle is mandatory and must match.
        assert!(user_handle_matches(owner, Some(owner.as_bytes()), true));
        assert!(!user_handle_matches(owner, Some(other.as_bytes()), true));
        assert!(!user_handle_matches(owner, None, true));
        assert!(!user_handle_matches(owner, Some(&[]), true));
        assert!(!user_handle_matches(owner, Some(b"short"), true));

        // Scoped: absence is fine (non-discoverable credentials), a wrong handle is not.
        assert!(user_handle_matches(owner, None, false));
        assert!(user_handle_matches(owner, Some(&[]), false));
        assert!(user_handle_matches(owner, Some(owner.as_bytes()), false));
        assert!(!user_handle_matches(owner, Some(other.as_bytes()), false));
        assert!(!user_handle_matches(owner, Some(b"short"), false));
    }

    // -- passkey names -------------------------------------------------------

    #[test]
    fn passkey_names_are_filtered() {
        let ok = |s: &str| sanitise_passkey_name(Some(s));

        assert_eq!(sanitise_passkey_name(None), "Passkey");
        assert_eq!(ok(""), "Passkey");
        assert_eq!(ok("   "), "Passkey");
        assert_eq!(ok("MacBook"), "MacBook");
        assert_eq!(ok("  My   iPhone  "), "My iPhone");
        assert_eq!(ok("Anna's YubiKey (5C)"), "Anna's YubiKey (5C)");
        // What an iPhone or Mac keyboard actually types: the smart quotes are substituted
        // for the straight ones by default, so these names must survive rather than coming
        // back as a 400 after the user has already passed the biometric check.
        assert_eq!(ok("Alice’s iPhone"), "Alice’s iPhone");
        assert_eq!(ok("‘Work’ YubiKey"), "‘Work’ YubiKey");
        assert_eq!(ok("“Home” iPad"), "“Home” iPad");
        assert_eq!(ok("Läptop Büro"), "Läptop Büro");
        assert_eq!(ok("Telefon 日本語"), "Telefon 日本語");
        assert_eq!(ok(&"a".repeat(64)), "a".repeat(64));
    }

    /// A name the label rules dislike must never cost the user their freshly created
    /// credential, so every disallowed character is dropped instead of refusing the
    /// registration. See the doc comment on [`sanitise_passkey_name`].
    #[test]
    fn a_disallowed_name_is_stripped_rather_than_refused() {
        assert_eq!(sanitise_passkey_name(Some("emoji 🔑")), "emoji");
        assert_eq!(sanitise_passkey_name(Some("🔑")), "Passkey");
        assert_eq!(sanitise_passkey_name(Some("a/b:c")), "abc");
        assert_eq!(sanitise_passkey_name(Some("quote\"")), "quote");
        assert_eq!(sanitise_passkey_name(Some("semi;colon")), "semicolon");
        // Dropping the emoji leaves the words on either side to be re-collapsed into one
        // space, rather than a double space with a hole in it.
        assert_eq!(sanitise_passkey_name(Some("Anna 🔑 iPhone")), "Anna iPhone");
        // Markup is dropped character by character, so no tag survives to be rendered.
        assert_eq!(
            sanitise_passkey_name(Some("<script>alert(1)</script>")),
            "scriptalert(1)script"
        );
        // Invisible characters go too: a zero-width space and a right-to-left override
        // could make one label read as another.
        assert_eq!(
            sanitise_passkey_name(Some("zero\u{200B}width")),
            "zerowidth"
        );
        assert_eq!(
            sanitise_passkey_name(Some("name\u{202E}gnp.exe")),
            "namegnp.exe"
        );
        assert_eq!(sanitise_passkey_name(Some("bell\u{7}")), "bell");
        // Line breaks are whitespace: collapsed into a space, not smuggled through.
        assert_eq!(sanitise_passkey_name(Some("two\nlines")), "two lines");
    }

    #[test]
    fn an_over_long_name_is_cut_to_length_not_refused() {
        // The tail is dropped, so the key is still registered under a usable label.
        assert_eq!(sanitise_passkey_name(Some(&"a".repeat(65))), "a".repeat(64));
        // Cutting on characters, not bytes, keeps multi-byte scripts intact.
        assert_eq!(
            sanitise_passkey_name(Some(&"日".repeat(70))),
            "日".repeat(64)
        );
        // A cut that lands on the separator does not leave a trailing space behind.
        let long = format!("{} bb", "a".repeat(63));
        assert_eq!(sanitise_passkey_name(Some(&long)), "a".repeat(63));
    }

    // -- challenge store -----------------------------------------------------

    #[test]
    fn challenge_ttl_outlives_the_browsers_timeout() {
        // The browser gives up after DEFAULT_AUTHENTICATOR_TIMEOUT. The store must not
        // expire a ceremony the browser would still let the user finish, but it must
        // not keep one alive for much longer either.
        assert!(CHALLENGE_TTL > DEFAULT_AUTHENTICATOR_TIMEOUT);
        assert!(CHALLENGE_TTL <= DEFAULT_AUTHENTICATOR_TIMEOUT + Duration::from_mins(1));
    }

    #[test]
    fn take_returns_the_state_once_and_then_nothing() {
        let store = ChallengeStore::new();
        let id = store.insert(dummy_registration_state(), Owner::User(7));

        assert!(store.take(&id, Some(7)).is_some());
        assert!(
            store.take(&id, Some(7)).is_none(),
            "a challenge must be single-use so replay is impossible"
        );
    }

    #[test]
    fn take_refuses_a_challenge_bound_to_another_user() {
        let store = ChallengeStore::new();
        let id = store.insert(dummy_registration_state(), Owner::User(7));

        assert!(
            store.take(&id, Some(8)).is_none(),
            "a challenge bound to user 7 must not be consumable by user 8"
        );
        assert!(
            store.take(&id, Some(7)).is_none(),
            "and the refusal must consume it, not leave it usable"
        );
    }

    #[test]
    fn take_refuses_an_anonymous_challenge_when_a_user_is_expected() {
        let store = ChallengeStore::new();
        let id = store.insert(dummy_registration_state(), Owner::Anonymous(ip(1)));
        assert!(store.take(&id, Some(3)).is_none());
    }

    #[test]
    fn take_refuses_a_user_challenge_when_anonymous_is_expected() {
        let store = ChallengeStore::new();
        let id = store.insert(dummy_registration_state(), Owner::User(3));
        assert!(store.take(&id, None).is_none());
    }

    #[test]
    fn take_of_an_unknown_id_returns_none() {
        let store = ChallengeStore::new();
        assert!(store.take(&Uuid::new_v4(), None).is_none());
    }

    #[test]
    fn expired_challenges_are_rejected() {
        let store = ChallengeStore::new();
        let stale = Instant::now()
            .checked_sub(CHALLENGE_TTL + Duration::from_secs(1))
            .expect("test clock must allow a subtraction longer than the TTL");
        let id = store.insert_with_created_at(
            dummy_registration_state(),
            Owner::Anonymous(ip(1)),
            stale,
        );
        assert!(
            store.take(&id, None).is_none(),
            "expired entries must not be usable"
        );
    }

    #[test]
    fn an_expired_challenge_is_dropped_by_the_next_insert() {
        // Memory stays bounded even if the background sweeper never runs: every insert
        // reclaims whatever has expired first. A stale entry is therefore stored, but
        // never survives the next insert.
        let store = ChallengeStore::new();
        let stale = Instant::now()
            .checked_sub(CHALLENGE_TTL + Duration::from_secs(1))
            .expect("test clock must allow a subtraction longer than the TTL");

        let dead = store.insert_with_created_at(
            dummy_registration_state(),
            Owner::Anonymous(ip(1)),
            stale,
        );
        assert_eq!(store.len(), 1, "the stale entry is stored to begin with");

        let fresh = store.insert(dummy_registration_state(), Owner::Anonymous(ip(2)));
        assert_eq!(
            store.len(),
            1,
            "the next insert must have reclaimed the stale entry"
        );
        assert!(
            store.take(&dead, None).is_none(),
            "the reclaimed ceremony must be gone"
        );
        assert!(store.take(&fresh, None).is_some());
    }

    #[test]
    fn store_evicts_oldest_when_the_anonymous_pool_is_full() {
        let store = ChallengeStore::new();
        // Spread over addresses so the per-address cap is not what is being tested.
        let addr = spread_addr;
        let mut ids = Vec::with_capacity(MAX_ANONYMOUS_CHALLENGES);
        for i in 0..MAX_ANONYMOUS_CHALLENGES {
            ids.push(store.insert(dummy_registration_state(), Owner::Anonymous(addr(i))));
        }
        assert_eq!(store.len_anonymous(), MAX_ANONYMOUS_CHALLENGES);

        let newest = store.insert(
            dummy_registration_state(),
            Owner::Anonymous(addr(MAX_ANONYMOUS_CHALLENGES)),
        );
        assert_eq!(
            store.len_anonymous(),
            MAX_ANONYMOUS_CHALLENGES,
            "the cap must hold"
        );
        assert!(
            store.take(&ids[0], None).is_none(),
            "the oldest entry must have been evicted"
        );
        assert!(
            store.take(&newest, None).is_some(),
            "the newest entry must survive"
        );
    }

    #[test]
    fn one_address_can_only_churn_its_own_challenges() {
        let store = ChallengeStore::new();
        let bystander = store.insert(dummy_registration_state(), Owner::Anonymous(ip(1)));

        let mut flood = Vec::new();
        for _ in 0..(MAX_ANONYMOUS_PER_IP * 3) {
            flood.push(store.insert(dummy_registration_state(), Owner::Anonymous(ip(99))));
        }

        assert!(
            store.take(&bystander, None).is_some(),
            "another address's ceremony must survive a flood"
        );
        assert!(
            store.take(&flood[0], None).is_none(),
            "the flooder's own oldest ceremonies are the ones evicted"
        );
        assert!(store.take(flood.last().unwrap(), None).is_some());
    }

    #[test]
    fn ipv6_addresses_are_grouped_by_their_64() {
        let a: IpAddr = "2001:db8:1:2:aaaa:bbbb:cccc:dddd".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(ip_bucket(a), ip_bucket(b));
        assert_ne!(ip_bucket(a), ip_bucket(c));

        let mapped: IpAddr = "::ffff:203.0.113.5".parse().unwrap();
        assert_eq!(ip_bucket(mapped), ip(5));
    }

    #[test]
    fn a_sign_in_flood_cannot_evict_a_registration() {
        let store = ChallengeStore::new();
        let registration = store.insert(dummy_registration_state(), Owner::User(42));

        for i in 0..(MAX_ANONYMOUS_CHALLENGES + 500) {
            store.insert(dummy_registration_state(), Owner::Anonymous(spread_addr(i)));
        }

        assert!(
            store.take(&registration, Some(42)).is_some(),
            "registrations live in their own pool"
        );
    }

    #[test]
    fn a_user_holds_one_registration_at_a_time() {
        let store = ChallengeStore::new();
        let first = store.insert(dummy_registration_state(), Owner::User(5));
        let second = store.insert(dummy_registration_state(), Owner::User(5));

        assert_eq!(store.len_registrations(), 1);
        assert!(
            store.take(&first, Some(5)).is_none(),
            "superseded by the newer attempt"
        );
        assert!(store.take(&second, Some(5)).is_some());
    }

    #[test]
    fn store_indexes_stay_consistent() {
        let store = ChallengeStore::new();
        let a = store.insert(dummy_registration_state(), Owner::Anonymous(ip(1)));
        let b = store.insert(dummy_registration_state(), Owner::User(9));
        store.take(&a, None);
        store.take(&b, Some(9));

        let inner = store.lock();
        assert!(inner.entries.is_empty());
        assert!(inner.anonymous.is_empty());
        assert!(inner.registrations.is_empty());
        assert!(inner.per_ip.is_empty());
        assert!(inner.per_user.is_empty());
    }

    #[test]
    fn a_poisoned_store_recovers_empty_instead_of_failing_silently() {
        let store = Arc::new(ChallengeStore::new());
        let id = store.insert(dummy_registration_state(), Owner::Anonymous(ip(1)));

        let poisoner = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.lock().unwrap();
            panic!("poison the mutex on purpose");
        })
        .join();
        assert!(store.inner.is_poisoned());

        // Recovers (discarding what it can no longer trust) and keeps working.
        assert!(store.take(&id, None).is_none());
        assert!(!store.inner.is_poisoned());
        let fresh = store.insert(dummy_registration_state(), Owner::Anonymous(ip(2)));
        assert!(store.take(&fresh, None).is_some());
    }

    // -- rate limiting and errors --------------------------------------------

    #[test]
    fn the_rate_limiter_is_per_address() {
        let guard = test_guard();
        let burst = RATE_LIMIT_BURST.get();
        for _ in 0..burst {
            assert!(guard.check(ip(1)).is_ok());
        }
        assert!(
            matches!(guard.check(ip(1)), Err(PasskeyHttpError::RateLimited)),
            "the burst must run out"
        );
        assert!(guard.check(ip(2)).is_ok(), "other addresses are unaffected");
    }

    #[test]
    fn http_errors_map_to_distinct_statuses() {
        assert_eq!(
            PasskeyHttpError::RateLimited.into_response().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            PasskeyHttpError::LimitReached.into_response().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            PasskeyHttpError::from(Error::Unauthorized)
                .into_response()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn counter_regression_is_reported_as_such() {
        assert!(matches!(
            PasskeyError::from_webauthn(&WebauthnError::CredentialPossibleCompromise),
            PasskeyError::PossibleCompromise
        ));
    }

    // -- response headers ----------------------------------------------------

    #[tokio::test]
    async fn passkey_responses_forbid_caching() {
        // A challenge, and on sign-in a session cookie, must never be stored by a
        // browser, a reverse proxy or any intermediary cache.
        let response = no_store(StatusCode::OK.into_response()).await;
        let value = |name: header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        assert_eq!(
            value(header::CACHE_CONTROL).as_deref(),
            Some("no-store, private")
        );
        assert_eq!(value(header::PRAGMA).as_deref(), Some("no-cache"));
    }

    // -- templates -----------------------------------------------------------

    /// Templates that predate the CSP and are already broken in a browser. The site is served
    /// with `script-src 'self'; style-src 'self'` and no nonce, so an inline `<script>` never runs,
    /// an inline event handler never fires and an inline `style=` is dropped — silently, and only
    /// in production. A subset check: removing an entry keeps it green, a new offender fails.
    const TEMPLATES_WITH_KNOWN_CSP_VIOLATIONS: &[&str] = &[
        // The cookie banner cannot be revealed or dismissed, and the `display:none` meant to
        // hide it is ignored, so it is stuck on every page.
        "layout_footer.html",
        // `style="display:none"` on a data carrier is ignored.
        "edit_deck.html",
        // `style="display:inline"` on a toggle form is ignored, so each toggle wraps.
        "manage_decks.html",
    ];

    /// Our templates only. `static/` is vendored third-party code (`CKEditor` and friends) whose
    /// markup we do not author or fix.
    fn collect_templates(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("the template directory must be readable");
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "static") {
                    continue;
                }
                collect_templates(&path, found);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "html")
            {
                found.push(path);
            }
        }
    }

    /// An attribute the policy refuses: the word boundary keeps `data-style=` out of it, and
    /// `script-src` covers `onclick`/`onload` just as much as a `<script>` block.
    fn refused_attribute() -> &'static regex::Regex {
        static ATTRIBUTE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        ATTRIBUTE.get_or_init(|| {
            regex::Regex::new(r#"(?i)(?:^|[\s"'<])(style|on[a-z]+)\s*="#).expect("a valid pattern")
        })
    }

    fn csp_violations(source: &str) -> Vec<String> {
        let mut reasons = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let number = index + 1;
            if let Some(found) = refused_attribute().captures(line) {
                let name = found
                    .get(1)
                    .map(|group| group.as_str().to_lowercase())
                    .unwrap_or_default();
                let kind = if name == "style" {
                    "style attribute"
                } else {
                    "event handler"
                };
                reasons.push(format!("line {number}: inline {kind} ({name}=)"));
            }

            // A <script> without src is inline, unless it is a data block such as
            // application/json: the browser never executes those.
            let mut rest = line;
            while let Some(start) = rest.find("<script") {
                let Some(offset) = rest[start..].find('>') else {
                    break;
                };
                let tag = &rest[start..=start + offset];
                let external = tag.contains("src=");
                let data_block = tag.contains("application/json")
                    || tag.contains("application/ld+json")
                    || tag.contains("text/template");
                if !external && !data_block {
                    reasons.push(format!("line {number}: inline <script>"));
                }
                rest = &rest[start + offset + 1..];
            }
        }
        reasons
    }

    #[test]
    fn templates_respect_the_content_security_policy() {
        let mut templates = Vec::new();
        collect_templates(std::path::Path::new("src/templates"), &mut templates);
        assert!(
            !templates.is_empty(),
            "expected to find templates to check; is the test run from the crate root?"
        );

        let mut unexpected = Vec::new();
        for template in &templates {
            let name = template
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            if TEMPLATES_WITH_KNOWN_CSP_VIOLATIONS.contains(&name.as_str()) {
                continue;
            }
            let source = std::fs::read_to_string(template).expect("a template must be readable");
            for reason in csp_violations(&source) {
                unexpected.push(format!("{name}: {reason}"));
            }
        }

        assert!(
            unexpected.is_empty(),
            "a template relies on inline CSS or an inline script. The CSP blocks both, and the \
             browser fails silently, so this only ever shows up in production:\n{}",
            unexpected.join("\n")
        );
    }

    #[test]
    fn passkey_pages_parse_and_render() {
        // Everything below mirrors how `main` builds Tera, because that is what
        // makes a template mistake fatal at startup. This test moves that failure
        // into CI instead of a deploy.
        let mut tera = tera::Tera::default();
        tera.register_filter("striptags", tera_contrib::regex::striptags);
        tera.register_filter("json_encode", tera_contrib::json::json_encode);
        tera.load_from_glob("src/templates/**/*.html")
            .expect("every template must parse");

        let user = serde_json::json!({ "id": 1, "username": "alice", "is_admin": false });

        // A signed-in user with one passkey, which is the state after registering.
        let mut with_passkey = tera::Context::new();
        with_passkey.insert("user", &user);
        let one_passkey = vec![serde_json::json!({
            "id": 1,
            "name": "MacBook",
            "created_at": "2026-09-29",
            "last_used_at": null,
        })];
        with_passkey.insert("passkeys", &one_passkey);
        with_passkey.insert("show_passkey_prompt", &false);
        tera.render("profile.html", &with_passkey)
            .expect("profile.html must render for a user with a passkey");
        tera.render("index.html", &with_passkey)
            .expect("index.html must render for a user with a passkey");

        // And the state every existing user is in today: no passkeys at all, so
        // the adoption card shows and the table is empty.
        let mut without_passkey = tera::Context::new();
        without_passkey.insert("user", &user);
        let no_passkeys: Vec<serde_json::Value> = Vec::new();
        without_passkey.insert("passkeys", &no_passkeys);
        without_passkey.insert("show_passkey_prompt", &true);
        tera.render("profile.html", &without_passkey)
            .expect("profile.html must render with no passkeys");
        tera.render("index.html", &without_passkey)
            .expect("index.html must render with the adoption card shown");

        // The login and signup pages, which the passkey button and nudge live on.
        let anonymous = tera::Context::new();
        tera.render("login.html", &anonymous)
            .expect("login.html must render");
        tera.render("signup.html", &anonymous)
            .expect("signup.html must render");

        // The passkey offer that follows sign-up, in both of its states: a session minted
        // moments ago renders no password field, an older one does.
        for show_passkey_password in [true, false] {
            let mut welcome = tera::Context::new();
            welcome.insert("user", &user);
            welcome.insert("show_passkey_password", &show_passkey_password);
            tera.render("welcome_passkey.html", &welcome).unwrap_or_else(|e| {
                panic!(
                    "welcome_passkey.html must render with show_passkey_password={show_passkey_password}: {e}"
                )
            });
        }

        // Every state of the pairing page. They are separate layouts rather than
        // one with bits hidden, so each has to be walked or a missing variable
        // would only show up when someone pairs a device. `code` is also a second
        // autofocus branch: present focuses Approve, absent focuses the field.
        for state in ["form", "malformed", "unknown", "approved", "denied"] {
            for code in ["ABC-234", ""] {
                let mut pairing = tera::Context::new();
                pairing.insert("user", &user);
                pairing.insert("state", &state);
                pairing.insert("code", &code);
                tera.render("pair.html", &pairing).unwrap_or_else(|e| {
                    panic!("pair.html must render in the {state} state with code {code:?}: {e}")
                });
            }
        }
    }

    // -- login page structure ------------------------------------------------

    #[test]
    fn login_ships_both_steps_so_it_works_without_scripting() {
        let mut tera = tera::Tera::default();
        tera.register_filter("striptags", tera_contrib::regex::striptags);
        tera.register_filter("json_encode", tera_contrib::json::json_encode);
        tera.load_from_glob("src/templates/**/*.html")
            .expect("every template must parse");

        let html = tera
            .render("login.html", &tera::Context::new())
            .expect("login.html must render");

        // Both steps are in the markup rather than one being built on demand, so the
        // form still submits in one piece when nothing can move between them.
        assert!(
            html.contains(r#"data-login-step="username""#)
                && html.contains(r#"data-login-step="verification""#),
            "both sign-in steps must be in the markup, otherwise a browser without \
             scripting has no way to reach the password field"
        );

        // The passkey is the way in, so it carries the filled styling and the password
        // is not on screen until it is asked for. A password form shown by default is
        // what made the passkey read as the optional route.
        assert!(
            html.contains(r#"class="auth-btn auth-btn--primary" data-passkey-action="sign-in""#),
            "the passkey button must be the primary action on the step"
        );

        // Step one starts with its nameless passkey button hidden, because only a device that
        // can find a credential of its own can answer a ceremony that names none; `passkeys.js`
        // reveals it after asking. Dropping the `hidden` attribute would put a button in front
        // of every visitor that can only open an empty prompt for most of them.
        assert!(
            html.contains(
                r#"class="auth-btn auth-btn--primary" data-passkey-action="sign-in-device" data-passkey-label hidden"#
            ),
            "step one must carry the nameless passkey button, hidden until the device answers for it"
        );
        // And the username keeps the plain styling for `passkeys.js` to trade places with that
        // button, so exactly one of the two is ever the filled action.
        assert!(
            html.contains(r#"class="auth-btn auth-btn--secondary" data-login-continue"#),
            "Continue must start secondary: passkeys.js promotes it only where the passkey \
             button is not shown"
        );

        assert!(
            html.contains("data-login-password-fields hidden"),
            "the password must stay off the step until it is asked for"
        );

        // "Keep me signed in" has to be reachable from both routes, so it is a child of the
        // form and not of either step. The passkey routes finish without ever showing step
        // two, and inside it the box was invisible to exactly the people taking them: their
        // session was then made non-persistent without them being shown a choice at all.
        let (_, after_step_one) = html
            .split_once("data-login-error")
            .expect("step one must carry its own message area");
        let (_, from_step_two) = html
            .split_once(r#"<div class="auth-step" data-login-step="verification""#)
            .expect("both sign-in steps must be in the markup");
        assert!(
            after_step_one.contains(r#"name="cookie""#)
                && !from_step_two.contains(r#"name="cookie""#),
            "the session choice must sit outside the steps, ahead of the action it applies to"
        );

        // Conditional UI is armed against this exact attribute, so losing it would
        // silently remove the no-typing route for a platform passkey.
        assert!(
            html.contains(r#"autocomplete="username webauthn""#),
            "the username field must stay eligible for conditional UI"
        );

        // No scripting means no ceremony at all, so the fallback has to hand the step
        // over to the password instead of leaving it behind the button that opens it.
        let noscript = html
            .split("<noscript>")
            .nth(1)
            .and_then(|rest| rest.split("</noscript>").next())
            .expect("login.html must carry a noscript fallback");
        assert!(
            noscript.contains("[data-login-password-fields]")
                && noscript.contains("[data-passkey-section]"),
            "a browser without scripting must be shown the password and not the \
             passkey affordances it cannot use"
        );
    }
}
