use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    /// Base URL of the Audiobookshelf instance, no trailing slash.
    pub abs_base_url: String,
    /// TCP address to listen on (e.g. "0.0.0.0:3456"). Takes precedence over PORT.
    pub bind: String,
    /// Path to the compiled Vue SPA static files.
    pub static_dir: String,
    /// When true, skip ABS token validation (useful for integration tests).
    pub bypass_auth: bool,
    /// Allowed cross-origin for CORS (e.g. "http://localhost:5173" for Vite dev).
    /// Omit in production — the SPA is served from the same origin.
    pub cors_origin: Option<String>,
    /// When false, the ABS username/password and API key sign-in routes are disabled.
    pub local_login: bool,
    /// SAML single sign-on. None when SAML is not configured.
    pub saml: Option<SamlConfig>,
}

#[derive(Clone)]
pub struct SamlConfig {
    /// Public URL of this party server, no trailing slash (e.g. "https://party.example.com").
    pub public_url: String,
    /// SP entity ID. Defaults to "<public_url>/auth/saml/metadata".
    pub entity_id: String,
    /// Where to load the IdP metadata XML from.
    pub idp_metadata: IdpMetadataSource,
    /// SAML attribute holding the ABS username, or "NameID" to use the subject NameID.
    pub username_attribute: String,
    /// ABS API key of an admin user, used to look up users and mint per-user API keys.
    pub abs_admin_api_key: String,
    /// Lifetime of the per-user ABS API keys minted after SAML sign-in, in seconds.
    pub key_ttl_secs: u64,
    /// Text of the sign-in button (e.g. "Sign in with Authentik").
    pub login_label: String,
}

// Hand-written so the admin key never ends up in logs.
impl std::fmt::Debug for SamlConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamlConfig")
            .field("public_url", &self.public_url)
            .field("entity_id", &self.entity_id)
            .field("idp_metadata", &self.idp_metadata)
            .field("username_attribute", &self.username_attribute)
            .field("abs_admin_api_key", &"<redacted>")
            .field("key_ttl_secs", &self.key_ttl_secs)
            .field("login_label", &self.login_label)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum IdpMetadataSource {
    Url(String),
    File(String),
}

/// Authentik's SAML property mapping for the username.
pub const AUTHENTIK_USERNAME_ATTRIBUTE: &str = "http://schemas.goauthentik.io/2021/02/saml/username";

fn env_flag(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

/// Reads NAME, or the contents of the file named by NAME_FILE (for Docker/systemd secrets).
fn env_or_file(name: &str) -> Option<String> {
    if let Ok(v) = env::var(name) {
        return Some(v.trim().to_string());
    }
    let path = env::var(format!("{name}_FILE")).ok()?;
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{name}_FILE: cannot read {path}: {e}"));
    Some(contents.trim().to_string())
}

impl SamlConfig {
    /// Returns None when SAML_IDP_METADATA_URL and SAML_IDP_METADATA_FILE are both unset.
    /// Panics on a half-finished SAML configuration rather than starting without it.
    fn from_env() -> Option<Self> {
        let idp_metadata = match (
            env::var("SAML_IDP_METADATA_URL").ok(),
            env::var("SAML_IDP_METADATA_FILE").ok(),
        ) {
            (Some(url), None) => IdpMetadataSource::Url(url),
            (None, Some(path)) => IdpMetadataSource::File(path),
            (None, None) => return None,
            (Some(_), Some(_)) => {
                panic!("set only one of SAML_IDP_METADATA_URL and SAML_IDP_METADATA_FILE")
            }
        };
        let public_url = env::var("PUBLIC_URL")
            .expect("PUBLIC_URL is required when SAML is enabled")
            .trim_end_matches('/')
            .to_string();
        let abs_admin_api_key = env_or_file("ABS_ADMIN_API_KEY")
            .filter(|k| !k.is_empty())
            .expect("ABS_ADMIN_API_KEY or ABS_ADMIN_API_KEY_FILE is required when SAML is enabled");
        let key_ttl_hours: u64 = env::var("SSO_KEY_TTL_HOURS")
            .map(|v| v.parse().expect("SSO_KEY_TTL_HOURS must be a whole number"))
            .unwrap_or(12);
        assert!(key_ttl_hours > 0, "SSO_KEY_TTL_HOURS must be at least 1");
        Some(Self {
            entity_id: env::var("SAML_SP_ENTITY_ID")
                .unwrap_or_else(|_| format!("{public_url}/auth/saml/metadata")),
            public_url,
            idp_metadata,
            username_attribute: env::var("SAML_USERNAME_ATTRIBUTE")
                .unwrap_or_else(|_| AUTHENTIK_USERNAME_ATTRIBUTE.into()),
            abs_admin_api_key,
            key_ttl_secs: key_ttl_hours * 3600,
            login_label: env::var("SAML_LOGIN_LABEL")
                .unwrap_or_else(|_| "Sign in with single sign-on".into()),
        })
    }

    pub fn acs_url(&self) -> String {
        format!("{}/auth/saml/acs", self.public_url)
    }
}

impl Config {
    pub fn from_env() -> Self {
        let bind = env::var("BIND").unwrap_or_else(|_| {
            env::var("PORT")
                .map(|p| format!("0.0.0.0:{p}"))
                .unwrap_or_else(|_| "0.0.0.0:3456".into())
        });
        let saml = SamlConfig::from_env();
        let local_login = env_flag("LOCAL_LOGIN", true);
        assert!(
            local_login || saml.is_some(),
            "LOCAL_LOGIN=0 needs SAML configured, or nobody can sign in"
        );
        Self {
            abs_base_url: env::var("ABS_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:13378".into()),
            bind,
            static_dir: env::var("STATIC_DIR").unwrap_or_else(|_| "./static".into()),
            bypass_auth: env_flag("BYPASS_AUTH", false),
            cors_origin: env::var("CORS_ORIGIN").ok(),
            local_login,
            saml,
        }
    }

    /// Constructs a test-friendly config. Available in all build profiles so
    /// integration tests (which are separate crates) can call it.
    pub fn for_test() -> Self {
        Self {
            abs_base_url: "http://abs.test".into(),
            bind: "127.0.0.1:0".into(),
            static_dir: "./static".into(),
            bypass_auth: true,
            cors_origin: None,
            local_login: true,
            saml: None,
        }
    }
}
