//! SAML 2.0 single sign-on (SP-initiated, HTTP-Redirect out, HTTP-POST back).
//!
//! The party server never has ABS passwords. After the IdP vouches for a user, the
//! server matches the SAML username to an ABS user and mints a short-lived ABS API key
//! for that user with the admin key. The browser then uses that key exactly like a
//! token from the password login, so each listener still streams from and saves
//! progress to their own ABS account.
//!
//! Hand-off to the browser goes through a one-time code (`/?sso=<code>`, redeemed by
//! POST), so the ABS key never appears in a URL or the browser history.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use dashmap::DashMap;
use rand::RngCore;
use samael::{
    crypto::AllowedSignatureAlgorithm,
    metadata::{EntityDescriptor, HTTP_POST_BINDING, HTTP_REDIRECT_BINDING},
    schema::Assertion,
    service_provider::{ServiceProvider, ServiceProviderBuilder},
};

use crate::{
    abs_client::AbsClient,
    config::{IdpMetadataSource, SamlConfig},
    AppState,
};

/// Prefix of every ABS API key this server mints. Logout and the expiry sweep only
/// ever touch keys whose name starts with it.
pub const KEY_PREFIX: &str = "abs-party-sso:";

const COOKIE: &str = "abs_party_saml";
/// How long a user has to finish signing in at the IdP.
const PENDING_TTL: Duration = Duration::from_secs(600);
/// How long the browser has to redeem the one-time code after the redirect back.
const GRANT_TTL: Duration = Duration::from_secs(60);
/// Cap on sign-ins in progress, so spamming /auth/saml/login can't eat memory.
const MAX_PENDING: usize = 10_000;

/// Serialises xmlsec signature checks. libxml2/xmlsec global state is not something
/// to bet on under concurrent use, and sign-ins are rare enough that this costs nothing.
static XML_LOCK: Mutex<()> = Mutex::new(());

pub struct Saml {
    sp: ServiceProvider,
    idp_sso_url: String,
    cfg: SamlConfig,
    /// AuthnRequest ID → when it was sent. Each ID is accepted once.
    pending: DashMap<String, Instant>,
    /// One-time code → minted ABS key, waiting for the browser to redeem it.
    grants: DashMap<String, (Instant, SsoGrant)>,
}

#[derive(Clone, serde::Serialize)]
pub struct SsoGrant {
    token: String,
    key_id: String,
    user: GrantUser,
}

#[derive(Clone, serde::Serialize)]
struct GrantUser {
    id: String,
    username: String,
}

/// Why a sign-in was refused; the code is shown to the user as `/?sso_error=<code>`.
#[derive(Debug)]
enum SsoError {
    /// The sign-in took too long, was already used, or the browser lost the cookie.
    Expired,
    /// The SAML response failed validation (signature, audience, times, InResponseTo).
    Invalid,
    NoUsername,
    NoAccount,
    Disabled,
    Root,
    /// Talking to ABS failed.
    Abs,
}

impl SsoError {
    fn code(&self) -> &'static str {
        match self {
            SsoError::Expired => "expired",
            SsoError::Invalid => "invalid",
            SsoError::NoUsername => "no_username",
            SsoError::NoAccount => "no_account",
            SsoError::Disabled => "disabled",
            SsoError::Root => "root",
            SsoError::Abs => "abs",
        }
    }
}

impl Saml {
    /// Loads the IdP metadata and builds the service provider. Fails, rather than
    /// starting insecurely, if the metadata has no signing certificate: samael skips
    /// signature verification entirely when it has no certificate to check against.
    pub async fn load(cfg: SamlConfig) -> Result<Self> {
        let xml = match &cfg.idp_metadata {
            IdpMetadataSource::Url(url) => {
                reqwest::get(url)
                    .await
                    .and_then(|r| r.error_for_status())
                    .with_context(|| format!("fetching IdP metadata from {url}"))?
                    .text()
                    .await?
            }
            IdpMetadataSource::File(path) => std::fs::read_to_string(path)
                .with_context(|| format!("reading IdP metadata from {path}"))?,
        };
        Self::from_metadata_xml(cfg, &xml)
    }

    pub fn from_metadata_xml(cfg: SamlConfig, xml: &str) -> Result<Self> {
        let idp_metadata: EntityDescriptor = xml
            .parse()
            .map_err(|e| anyhow!("parsing IdP metadata: {e}"))?;

        let sp = ServiceProviderBuilder::default()
            .entity_id(cfg.entity_id.clone())
            .metadata_url(format!("{}/auth/saml/metadata", cfg.public_url))
            .acs_url(cfg.acs_url())
            .slo_url(None::<String>)
            .idp_metadata(idp_metadata)
            .allowed_signature_algorithms(Some(vec![
                AllowedSignatureAlgorithm::RsaSha256,
                AllowedSignatureAlgorithm::RsaSha384,
                AllowedSignatureAlgorithm::RsaSha512,
                AllowedSignatureAlgorithm::EcdsaSha256,
                AllowedSignatureAlgorithm::EcdsaSha384,
                AllowedSignatureAlgorithm::EcdsaSha512,
            ]))
            .build()
            .map_err(|e| anyhow!("building SAML service provider: {e}"))?;

        match sp.idp_signing_certs() {
            Ok(Some(certs)) if !certs.is_empty() => {}
            Ok(_) => bail!(
                "IdP metadata has no signing certificate; refusing to accept unsigned SAML. \
                 In Authentik, set a signing certificate on the SAML provider."
            ),
            Err(e) => bail!("IdP signing certificate unreadable: {e}"),
        }
        let idp_sso_url = sp
            .sso_binding_location(HTTP_REDIRECT_BINDING)
            .ok_or_else(|| anyhow!("IdP metadata has no HTTP-Redirect SingleSignOnService"))?;

        Ok(Self {
            sp,
            idp_sso_url,
            cfg,
            pending: DashMap::new(),
            grants: DashMap::new(),
        })
    }

    fn purge_expired(&self) {
        self.pending.retain(|_, sent| sent.elapsed() < PENDING_TTL);
        self.grants
            .retain(|_, (made, _)| made.elapsed() < GRANT_TTL);
    }

    fn secure_cookie(&self) -> bool {
        self.cfg.public_url.starts_with("https://")
    }

    fn username_from(&self, assertion: &Assertion) -> Option<String> {
        let attr = self.cfg.username_attribute.as_str();
        let value = if attr.eq_ignore_ascii_case("NameID") {
            assertion
                .subject
                .as_ref()
                .and_then(|s| s.name_id.as_ref())
                .map(|n| n.value.clone())
        } else {
            assertion
                .attribute_statements
                .iter()
                .flatten()
                .flat_map(|stmt| &stmt.attributes)
                .filter(|a| {
                    a.name.as_deref() == Some(attr) || a.friendly_name.as_deref() == Some(attr)
                })
                .flat_map(|a| &a.values)
                .find_map(|v| v.value.clone())
        };
        value
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// Finds the ABS user called `username` and mints an API key that acts as them.
    async fn mint_key(&self, abs: &Arc<AbsClient>, username: &str) -> Result<SsoGrant, SsoError> {
        let admin = &self.cfg.abs_admin_api_key;
        let users = abs.list_users(admin).await.map_err(|e| {
            tracing::error!("SAML: listing ABS users with the admin key failed: {e:#}");
            SsoError::Abs
        })?;
        // ABS usernames are case-insensitive (it lowercases them on login).
        let wanted = username.to_lowercase();
        let mut matches = users
            .into_iter()
            .filter(|u| u.username.to_lowercase() == wanted);
        let user = match (matches.next(), matches.next()) {
            (Some(u), None) => u,
            (None, _) => return Err(SsoError::NoAccount),
            (Some(_), Some(_)) => {
                tracing::warn!("SAML: more than one ABS user matches {username:?}, refusing");
                return Err(SsoError::NoAccount);
            }
        };
        if !user.is_active {
            return Err(SsoError::Disabled);
        }
        // An SSO path to the root account would make the IdP account the root key.
        if user.user_type == "root" {
            return Err(SsoError::Root);
        }

        let name = format!(
            "{KEY_PREFIX}{}:{}",
            user.username,
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        );
        let key = abs
            .create_api_key(admin, &name, &user.id, self.cfg.key_ttl_secs)
            .await
            .map_err(|e| {
                tracing::error!(
                    "SAML: minting an ABS API key for {:?} failed: {e:#}",
                    user.username
                );
                SsoError::Abs
            })?;
        tracing::info!("SAML: signed in {:?}, ABS key {}", user.username, key.id);

        // Expired keys stay in ABS (it only deactivates them), so tidy ours up now and then.
        let abs = abs.clone();
        let admin = admin.clone();
        tokio::spawn(async move { sweep_expired_keys(&abs, &admin).await });

        Ok(SsoGrant {
            token: key.api_key,
            key_id: key.id,
            user: GrantUser {
                id: user.id,
                username: user.username,
            },
        })
    }
}

async fn sweep_expired_keys(abs: &AbsClient, admin_key: &str) {
    let keys = match abs.list_api_keys(admin_key).await {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("SAML: listing ABS API keys for cleanup failed: {e:#}");
            return;
        }
    };
    let now = chrono::Utc::now();
    for key in keys {
        let expired = key.expires_at.is_some_and(|t| t < now);
        if key.name.starts_with(KEY_PREFIX) && expired {
            if let Err(e) = abs.delete_api_key(admin_key, &key.id).await {
                tracing::warn!("SAML: deleting expired ABS key {} failed: {e:#}", key.id);
            }
        }
    }
}

fn random_code() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn cookie_header(saml: &Saml, value: &str, max_age: u64) -> String {
    // The IdP's POST back is a cross-site navigation, so the cookie must be
    // SameSite=None, which browsers only accept with Secure (https).
    let attrs = if saml.secure_cookie() {
        "SameSite=None; Secure"
    } else {
        "SameSite=Lax"
    };
    format!("{COOKIE}={value}; Path=/auth/saml; Max-Age={max_age}; HttpOnly; {attrs}")
}

fn not_enabled() -> Response {
    (StatusCode::NOT_FOUND, "SAML sign-in is not enabled").into_response()
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    resp
}

// --- Handlers ---

/// GET /auth/saml/login: send the browser to the IdP with a fresh AuthnRequest.
pub async fn login(State(state): State<AppState>) -> Response {
    let Some(saml) = state.saml.as_ref() else {
        return not_enabled();
    };
    saml.purge_expired();
    if saml.pending.len() >= MAX_PENDING {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many sign-ins in progress, try again shortly",
        )
            .into_response();
    }
    let request = match saml.sp.make_authentication_request(&saml.idp_sso_url) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("SAML: building AuthnRequest failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let url = match request.redirect("") {
        Ok(Some(url)) => url,
        other => {
            tracing::error!(
                "SAML: building redirect failed: {:?}",
                other.err().map(|e| e.to_string())
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    saml.pending.insert(request.id.clone(), Instant::now());

    let mut resp = Redirect::to(url.as_str()).into_response();
    if let Ok(v) = cookie_header(saml, &request.id, PENDING_TTL.as_secs()).parse() {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    no_store(resp)
}

#[derive(serde::Deserialize)]
pub struct AcsForm {
    #[serde(rename = "SAMLResponse")]
    saml_response: String,
}

/// POST /auth/saml/acs: the IdP posts the signed SAML response here.
pub async fn acs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AcsForm>,
) -> Response {
    let Some(saml) = state.saml.clone() else {
        return not_enabled();
    };
    let outcome = acs_inner(&state, &saml, &headers, form.saml_response).await;

    let target = match outcome {
        Ok(grant) => {
            let code = random_code();
            saml.grants.insert(code.clone(), (Instant::now(), grant));
            format!("/?sso={code}")
        }
        Err(e) => format!("/?sso_error={}", e.code()),
    };
    let mut resp = Redirect::to(&target).into_response();
    if let Ok(v) = cookie_header(&saml, "", 0).parse() {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    no_store(resp)
}

async fn acs_inner(
    state: &AppState,
    saml: &Arc<Saml>,
    headers: &HeaderMap,
    saml_response: String,
) -> Result<SsoGrant, SsoError> {
    // The cookie ties the response to the browser that started the sign-in, which
    // stops someone pushing their own valid response into another person's browser.
    let request_id = read_cookie(headers, COOKIE)
        .ok_or(SsoError::Expired)?
        .to_string();
    let (_, sent) = saml.pending.remove(&request_id).ok_or(SsoError::Expired)?;
    if sent.elapsed() >= PENDING_TTL {
        return Err(SsoError::Expired);
    }

    let parser = saml.clone();
    let assertion = tokio::task::spawn_blocking(move || {
        let _guard = XML_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        parser
            .sp
            .parse_base64_response(saml_response.trim(), Some(&[request_id.as_str()]))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| SsoError::Invalid)?
    .map_err(|e| {
        tracing::warn!("SAML: rejected response: {e}");
        SsoError::Invalid
    })?;

    let username = saml.username_from(&assertion).ok_or_else(|| {
        tracing::warn!(
            "SAML: assertion has no {:?} value; check the provider's property mappings",
            saml.cfg.username_attribute
        );
        SsoError::NoUsername
    })?;
    saml.mint_key(&state.abs, &username).await.inspect_err(|e| {
        tracing::info!("SAML: refused {username:?}: {e:?}");
    })
}

#[derive(serde::Deserialize)]
pub struct RedeemBody {
    code: String,
}

/// POST /api/auth/sso/redeem: swap the one-time code for the minted ABS key.
pub async fn redeem(State(state): State<AppState>, Json(body): Json<RedeemBody>) -> Response {
    let Some(saml) = state.saml.as_ref() else {
        return not_enabled();
    };
    let grant = saml
        .grants
        .remove(&body.code)
        .filter(|(_, (made, _))| made.elapsed() < GRANT_TTL)
        .map(|(_, (_, grant))| grant);
    match grant {
        Some(grant) => no_store(Json(grant).into_response()),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Sign-in expired" })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct LogoutBody {
    key_id: String,
}

/// POST /api/auth/sso/logout: delete the caller's own SSO key from ABS.
/// The Bearer token proves who is asking; only a key minted by this server for that
/// same ABS user can be deleted.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LogoutBody>,
) -> Response {
    let Some(saml) = state.saml.as_ref() else {
        return not_enabled();
    };
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    else {
        return (StatusCode::UNAUTHORIZED, "Missing Bearer token").into_response();
    };
    let me = match state.abs.me(token).await {
        Ok(me) => me,
        Err(_) => return (StatusCode::UNAUTHORIZED, "Token not accepted by ABS").into_response(),
    };
    let admin = &saml.cfg.abs_admin_api_key;
    let keys = match state.abs.list_api_keys(admin).await {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("SAML logout: listing ABS keys failed: {e:#}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let own_key = keys.iter().find(|k| {
        k.id == body.key_id
            && k.name.starts_with(KEY_PREFIX)
            && k.user_id.as_deref() == Some(me.id.as_str())
    });
    if let Some(key) = own_key {
        if let Err(e) = state.abs.delete_api_key(admin, &key.id).await {
            tracing::error!("SAML logout: deleting ABS key {} failed: {e:#}", key.id);
            return StatusCode::BAD_GATEWAY.into_response();
        }
        tracing::info!(
            "SAML: {:?} signed out, ABS key {} deleted",
            me.username,
            key.id
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

/// GET /auth/saml/metadata: SP metadata, for importing into the IdP.
/// Written by hand because samael's generator insists on a logout URL, which this
/// server does not implement.
pub async fn metadata(State(state): State<AppState>) -> Response {
    let Some(saml) = state.saml.as_ref() else {
        return not_enabled();
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{entity}">
  <md:SPSSODescriptor AuthnRequestsSigned="false" WantAssertionsSigned="true" protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:AssertionConsumerService Binding="{post}" Location="{acs}" index="0" isDefault="true"/>
  </md:SPSSODescriptor>
</md:EntityDescriptor>
"#,
        entity = xml_escape(&saml.cfg.entity_id),
        post = HTTP_POST_BINDING,
        acs = xml_escape(&saml.cfg.acs_url()),
    );
    (
        [(header::CONTENT_TYPE, "application/samlmetadata+xml")],
        xml,
    )
        .into_response()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
