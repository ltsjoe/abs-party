//! SAML sign-in, end to end: a real RSA-signed IdP response against a mock ABS.

use std::sync::{Arc, Mutex};

use abs_party::{
    build_app_with_saml,
    config::{Config, IdpMetadataSource, SamlConfig, AUTHENTIK_USERNAME_ATTRIBUTE},
    saml::{Saml, KEY_PREFIX},
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, Request, StatusCode},
    routing::{delete, get},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use samael::{
    crypto::{CertificateDer, Crypto, CryptoProvider},
    idp::{
        response_builder::{build_response_template, ResponseAttribute},
        sp_extractor::RequiredAttribute,
        CertificateParams, IdentityProvider, KeyType, Rsa,
    },
    traits::ToXml,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN_KEY: &str = "admin-key-for-tests";
const PUBLIC_URL: &str = "https://party.test";
const IDP_ENTITY: &str = "https://auth.test/application/saml/abs-party/";
const SHA1_DIGEST: &str = "http://www.w3.org/2000/09/xmldsig#sha1";
const SHA256_DIGEST: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
const IDP_SSO: &str = "https://auth.test/application/saml/abs-party/sso/binding/redirect/";

// --- Mock ABS ---

#[derive(Default)]
struct MockAbs {
    created: Vec<Value>,
    deleted: Vec<String>,
    keys: Vec<Value>,
}

type Mock = Arc<Mutex<MockAbs>>;

fn is_admin(h: &HeaderMap) -> bool {
    h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
        == Some(&format!("Bearer {ADMIN_KEY}"))
}

async fn spawn_mock_abs() -> (String, Mock) {
    let mock: Mock = Arc::default();
    let app = Router::new()
        .route(
            "/api/users",
            get(|h: HeaderMap| async move {
                if !is_admin(&h) {
                    return Err(StatusCode::FORBIDDEN);
                }
                Ok(Json(json!({ "users": [
                    { "id": "u-alice", "username": "Alice", "type": "user", "isActive": true },
                    { "id": "u-bob", "username": "bob", "type": "user", "isActive": false },
                    { "id": "u-root", "username": "root", "type": "root", "isActive": true },
                ]})))
            }),
        )
        .route(
            "/api/api-keys",
            get(|State(m): State<Mock>, h: HeaderMap| async move {
                if !is_admin(&h) {
                    return Err(StatusCode::FORBIDDEN);
                }
                Ok(Json(json!({ "apiKeys": m.lock().unwrap().keys.clone() })))
            })
            .post(
                |State(m): State<Mock>, h: HeaderMap, Json(body): Json<Value>| async move {
                    if !is_admin(&h) {
                        return Err(StatusCode::FORBIDDEN);
                    }
                    let mut m = m.lock().unwrap();
                    let id = format!("key-{}", m.created.len() + 1);
                    m.keys.push(json!({
                        "id": id, "name": body["name"], "userId": body["userId"],
                        "expiresAt": "2099-01-01T00:00:00.000Z",
                    }));
                    m.created.push(body.clone());
                    Ok(Json(json!({ "apiKey": {
                        "id": id, "apiKey": format!("jwt-for-{}", body["userId"].as_str().unwrap()),
                        "name": body["name"], "userId": body["userId"],
                    }})))
                },
            ),
        )
        .route(
            "/api/api-keys/:id",
            delete(
                |State(m): State<Mock>, h: HeaderMap, Path(id): Path<String>| async move {
                    if !is_admin(&h) {
                        return StatusCode::FORBIDDEN;
                    }
                    let mut m = m.lock().unwrap();
                    m.keys.retain(|k| k["id"] != id);
                    m.deleted.push(id);
                    StatusCode::OK
                },
            ),
        )
        .route(
            "/api/me",
            get(|h: HeaderMap| async move {
                match h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
                    Some("Bearer jwt-for-u-alice") => {
                        Ok(Json(json!({ "id": "u-alice", "username": "Alice" })))
                    }
                    Some("Bearer jwt-for-other") => {
                        Ok(Json(json!({ "id": "u-other", "username": "other" })))
                    }
                    _ => Err(StatusCode::UNAUTHORIZED),
                }
            }),
        )
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

// --- Test IdP ---

struct TestIdp {
    idp: IdentityProvider,
    cert: CertificateDer,
}

impl TestIdp {
    fn new() -> Self {
        let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
        let cert = idp
            .create_certificate(&CertificateParams {
                common_name: "auth.test",
                issuer_name: "auth.test",
                days_until_expiration: 2,
            })
            .unwrap();
        Self { idp, cert }
    }

    fn metadata(&self, with_cert: bool) -> String {
        let key = if with_cert {
            format!(
                r#"<md:KeyDescriptor use="signing"><ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:X509Data><ds:X509Certificate>{}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>"#,
                STANDARD.encode(self.cert.der_data())
            )
        } else {
            String::new()
        };
        format!(
            r#"<?xml version="1.0"?><md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY}"><md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">{key}<md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="{IDP_SSO}"/></md:IDPSSODescriptor></md:EntityDescriptor>"#
        )
    }

    /// A response the way Authentik sends one: username in the goauthentik attribute.
    fn response_xml(&self, in_response_to: &str, username: &str) -> String {
        let attrs = [ResponseAttribute {
            required_attribute: RequiredAttribute {
                name: AUTHENTIK_USERNAME_ATTRIBUTE.into(),
                format: Some("urn:oasis:names:tc:SAML:2.0:attrname-format:basic".into()),
            },
            value: username,
        }];
        let mut resp = build_response_template(
            &self.cert,
            "opaque-subject-id",
            &format!("{PUBLIC_URL}/auth/saml/metadata"),
            IDP_ENTITY,
            &format!("{PUBLIC_URL}/auth/saml/acs"),
            in_response_to,
            &attrs,
        );
        // samael's template leaves NotOnOrAfter off, which its SP side (rightly) rejects.
        let assertion = resp.assertion.as_mut().unwrap();
        for sc in assertion
            .subject
            .as_mut()
            .unwrap()
            .subject_confirmations
            .as_mut()
            .unwrap()
        {
            sc.subject_confirmation_data
                .as_mut()
                .unwrap()
                .not_on_or_after = Some(chrono::Utc::now() + chrono::Duration::minutes(5));
        }
        resp.to_string().unwrap()
    }

    /// Signs with a SHA-256 digest, as Authentik does by default.
    fn sign(&self, xml: &str) -> String {
        self.sign_raw(&xml.replace(SHA1_DIGEST, SHA256_DIGEST))
    }

    /// Signs with whatever digest the template names (samael's default is SHA-1).
    fn sign_raw(&self, xml: &str) -> String {
        assert!(xml.contains("DigestMethod"));
        Crypto::sign_xml(xml, &self.idp.export_private_key_der().unwrap()).unwrap()
    }

    fn signed_response(&self, in_response_to: &str, username: &str) -> String {
        STANDARD.encode(self.sign(&self.response_xml(in_response_to, username)))
    }
}

// --- Harness ---

struct Harness {
    app: Router,
    mock: Mock,
    idp: TestIdp,
}

fn saml_config(abs_url: &str) -> (Config, SamlConfig) {
    let mut cfg = Config::for_test();
    cfg.abs_base_url = abs_url.into();
    let saml = SamlConfig {
        public_url: PUBLIC_URL.into(),
        entity_id: format!("{PUBLIC_URL}/auth/saml/metadata"),
        idp_metadata: IdpMetadataSource::File("unused".into()),
        username_attribute: AUTHENTIK_USERNAME_ATTRIBUTE.into(),
        abs_admin_api_key: ADMIN_KEY.into(),
        key_ttl_secs: 12 * 3600,
        login_label: "Sign in with Authentik".into(),
    };
    cfg.saml = Some(saml.clone());
    (cfg, saml)
}

async fn harness() -> Harness {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("abs_party=debug")
        .try_init();
    let (abs_url, mock) = spawn_mock_abs().await;
    let idp = TestIdp::new();
    let (cfg, saml_cfg) = saml_config(&abs_url);
    let saml = Saml::from_metadata_xml(saml_cfg, &idp.metadata(true)).unwrap();
    Harness {
        app: build_app_with_saml(cfg, Some(Arc::new(saml))),
        mock,
        idp,
    }
}

impl Harness {
    /// Starts a sign-in; returns the AuthnRequest ID (from the cookie) and the IdP redirect.
    async fn start_login(&self) -> (String, String) {
        let resp = self
            .app
            .clone()
            .oneshot(
                Request::get("/auth/saml/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string();
        let cookie = resp.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("Secure")
                && cookie.contains("SameSite=None")
        );
        let id = cookie
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string();
        (id, location)
    }

    /// Posts a SAML response to the ACS; returns where the browser is sent next.
    async fn post_acs(&self, cookie_id: Option<&str>, saml_response: &str) -> String {
        let body = format!("SAMLResponse={}", urlencode(saml_response));
        let mut req = Request::post("/auth/saml/acs")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(id) = cookie_id {
            req = req.header(header::COOKIE, format!("other=1; abs_party_saml={id}"));
        }
        let resp = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        resp.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    async fn post_json(&self, uri: &str, bearer: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut req = Request::post(uri).header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = bearer {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let resp = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// --- Tests ---

#[tokio::test]
async fn full_sign_in_mints_a_key_for_the_matching_abs_user() {
    let h = harness().await;
    let (id, location) = h.start_login().await;
    assert!(location.starts_with(IDP_SSO) && location.contains("SAMLRequest="));

    // Username case differs from ABS ("Alice"): ABS usernames are case-insensitive.
    let next = h
        .post_acs(Some(&id), &h.idp.signed_response(&id, "alice"))
        .await;
    let code = next.strip_prefix("/?sso=").expect(&next);

    let (status, grant) = h
        .post_json("/api/auth/sso/redeem", None, json!({ "code": code }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(grant["token"], "jwt-for-u-alice");
    assert_eq!(grant["key_id"], "key-1");
    assert_eq!(grant["user"]["username"], "Alice");

    let created = h.mock.lock().unwrap().created.clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["userId"], "u-alice");
    assert_eq!(created[0]["isActive"], true);
    assert_eq!(created[0]["expiresIn"], 12 * 3600);
    assert!(created[0]["name"].as_str().unwrap().starts_with(KEY_PREFIX));

    // The code is single-use.
    let (status, _) = h
        .post_json("/api/auth/sso/redeem", None, json!({ "code": code }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn replayed_response_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    let resp = h.idp.signed_response(&id, "alice");
    assert!(h.post_acs(Some(&id), &resp).await.starts_with("/?sso="));
    assert_eq!(h.post_acs(Some(&id), &resp).await, "/?sso_error=expired");
    assert_eq!(h.mock.lock().unwrap().created.len(), 1);
}

#[tokio::test]
async fn response_without_the_sign_in_cookie_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    assert_eq!(
        h.post_acs(None, &h.idp.signed_response(&id, "alice")).await,
        "/?sso_error=expired"
    );
}

#[tokio::test]
async fn response_for_another_sign_in_is_refused() {
    // Login CSRF: a valid response the attacker got for their own sign-in, pushed
    // into a victim's browser that has a different sign-in pending.
    let h = harness().await;
    let (attacker_id, _) = h.start_login().await;
    let (victim_id, _) = h.start_login().await;
    let resp = h.idp.signed_response(&attacker_id, "alice");
    assert_eq!(
        h.post_acs(Some(&victim_id), &resp).await,
        "/?sso_error=invalid"
    );
    assert!(h.mock.lock().unwrap().created.is_empty());
}

#[tokio::test]
async fn unsigned_response_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    let unsigned = STANDARD.encode(h.idp.response_xml(&id, "alice"));
    assert_eq!(
        h.post_acs(Some(&id), &unsigned).await,
        "/?sso_error=invalid"
    );
}

#[tokio::test]
async fn tampered_response_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    let signed = h.idp.sign(&h.idp.response_xml(&id, "alice"));
    assert!(signed.contains(">alice<"));
    let tampered = STANDARD.encode(signed.replace(">alice<", ">root<"));
    assert_eq!(
        h.post_acs(Some(&id), &tampered).await,
        "/?sso_error=invalid"
    );
    assert!(h.mock.lock().unwrap().created.is_empty());
}

#[tokio::test]
async fn sha1_digest_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    let xml = h.idp.response_xml(&id, "alice");
    assert!(xml.contains(SHA1_DIGEST));
    let weak = STANDARD.encode(h.idp.sign_raw(&xml));
    assert_eq!(h.post_acs(Some(&id), &weak).await, "/?sso_error=invalid");
}

#[tokio::test]
async fn response_signed_by_another_key_is_refused() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    let impostor = TestIdp::new();
    assert_eq!(
        h.post_acs(Some(&id), &impostor.signed_response(&id, "alice"))
            .await,
        "/?sso_error=invalid"
    );
}

#[tokio::test]
async fn users_who_cannot_sign_in_are_refused_without_a_key() {
    let h = harness().await;
    for (user, err) in [
        ("nobody", "no_account"),
        ("bob", "disabled"),
        ("root", "root"),
    ] {
        let (id, _) = h.start_login().await;
        let next = h
            .post_acs(Some(&id), &h.idp.signed_response(&id, user))
            .await;
        assert_eq!(next, format!("/?sso_error={err}"), "user {user}");
    }
    assert!(h.mock.lock().unwrap().created.is_empty());
}

#[tokio::test]
async fn metadata_without_a_signing_certificate_is_rejected_at_startup() {
    let idp = TestIdp::new();
    let (_, saml_cfg) = saml_config("http://unused");
    let err = Saml::from_metadata_xml(saml_cfg, &idp.metadata(false))
        .err()
        .unwrap();
    assert!(err.to_string().contains("no signing certificate"), "{err}");
}

#[tokio::test]
async fn logout_deletes_only_the_callers_own_key() {
    let h = harness().await;
    let (id, _) = h.start_login().await;
    h.post_acs(Some(&id), &h.idp.signed_response(&id, "alice"))
        .await;

    // Someone else's valid token can't delete Alice's key.
    let (status, _) = h
        .post_json(
            "/api/auth/sso/logout",
            Some("jwt-for-other"),
            json!({ "key_id": "key-1" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(h.mock.lock().unwrap().deleted.is_empty());

    // No token, no deal.
    let (status, _) = h
        .post_json("/api/auth/sso/logout", None, json!({ "key_id": "key-1" }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = h
        .post_json(
            "/api/auth/sso/logout",
            Some("jwt-for-u-alice"),
            json!({ "key_id": "key-1" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(h.mock.lock().unwrap().deleted, vec!["key-1".to_string()]);
}

#[tokio::test]
async fn sp_metadata_and_config_advertise_saml() {
    let h = harness().await;
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::get("/auth/saml/metadata")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let xml = String::from_utf8(xml.to_vec()).unwrap();
    assert!(
        xml.contains(&format!(r#"Location="{PUBLIC_URL}/auth/saml/acs""#)),
        "{xml}"
    );
    assert!(
        xml.contains(&format!(r#"entityID="{PUBLIC_URL}/auth/saml/metadata""#)),
        "{xml}"
    );
    assert!(xml.contains(r#"WantAssertionsSigned="true""#), "{xml}");

    let resp = h
        .app
        .clone()
        .oneshot(Request::get("/api/config").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let cfg: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(cfg["saml_login"], true);
    assert_eq!(cfg["local_login"], true);
    assert_eq!(cfg["saml_login_label"], "Sign in with Authentik");
}

#[tokio::test]
async fn local_login_can_be_switched_off() {
    let (abs_url, _mock) = spawn_mock_abs().await;
    let idp = TestIdp::new();
    let (mut cfg, saml_cfg) = saml_config(&abs_url);
    cfg.local_login = false;
    let saml = Saml::from_metadata_xml(saml_cfg, &idp.metadata(true)).unwrap();
    let app = build_app_with_saml(cfg, Some(Arc::new(saml)));
    let resp = app
        .oneshot(
            Request::post("/api/abs/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"username":"a","password":"b"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::OK);
}
