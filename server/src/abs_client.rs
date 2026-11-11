use anyhow::Result;
use reqwest::{Client, header};

use crate::models::{AbsApiKey, AbsApiKeyCreated, AbsLoginResponse, AbsMeResponse, AbsUserSummary};

/// True when ABS answered 401 or 403, i.e. it refused the token itself (expired,
/// deleted or never valid), as opposed to ABS being unreachable or erroring.
pub fn is_auth_refusal(err: &anyhow::Error) -> bool {
    err.downcast_ref::<reqwest::Error>()
        .and_then(|e| e.status())
        .is_some_and(|s| s == reqwest::StatusCode::UNAUTHORIZED || s == reqwest::StatusCode::FORBIDDEN)
}

pub struct AbsClient {
    client: Client,
    base_url: String,
}

impl AbsClient {
    pub fn new(base_url: String) -> Self {
        let mut default_headers = header::HeaderMap::new();
        default_headers.insert(header::USER_AGENT, header::HeaderValue::from_static("abs-party/1.0"));
        default_headers.insert(header::ACCEPT, header::HeaderValue::from_static("application/json"));
        Self {
            client: Client::builder()
                .default_headers(default_headers)
                .build()
                .expect("failed to build reqwest client"),
            base_url,
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AbsLoginResponse> {
        let resp = self
            .client
            .post(format!("{}/login", self.base_url))
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await?
            .error_for_status()?
            .json::<AbsLoginResponse>()
            .await?;
        Ok(resp)
    }

    pub async fn me(&self, token: &str) -> Result<AbsMeResponse> {
        let resp = self
            .client
            .get(format!("{}/api/me", self.base_url))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json::<AbsMeResponse>()
            .await?;
        Ok(resp)
    }

    pub async fn proxy_raw(
        &self,
        token: &str,
        method: reqwest::Method,
        url: &str,
        body: bytes::Bytes,
    ) -> Result<(u16, bytes::Bytes, Option<String>)> {
        let mut builder = self.client.request(method, url).bearer_auth(token);
        if !body.is_empty() {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let resp = builder.send().await?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = resp.bytes().await?;
        Ok((status, bytes, content_type))
    }

    // --- Admin calls, made with the admin API key during SAML sign-in ---

    pub async fn list_users(&self, admin_key: &str) -> Result<Vec<AbsUserSummary>> {
        #[derive(serde::Deserialize)]
        struct Users {
            users: Vec<AbsUserSummary>,
        }
        let resp = self
            .client
            .get(format!("{}/api/users", self.base_url))
            .bearer_auth(admin_key)
            .send()
            .await?
            .error_for_status()?
            .json::<Users>()
            .await?;
        Ok(resp.users)
    }

    /// Mints an active API key that acts as `user_id` and expires after `expires_in_secs`.
    pub async fn create_api_key(
        &self,
        admin_key: &str,
        name: &str,
        user_id: &str,
        expires_in_secs: u64,
    ) -> Result<AbsApiKeyCreated> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Created {
            api_key: AbsApiKeyCreated,
        }
        let resp = self
            .client
            .post(format!("{}/api/api-keys", self.base_url))
            .bearer_auth(admin_key)
            // ABS creates keys inactive unless told otherwise.
            .json(&serde_json::json!({
                "name": name,
                "userId": user_id,
                "expiresIn": expires_in_secs,
                "isActive": true,
            }))
            .send()
            .await?
            .error_for_status()?
            .json::<Created>()
            .await?;
        Ok(resp.api_key)
    }

    pub async fn list_api_keys(&self, admin_key: &str) -> Result<Vec<AbsApiKey>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Keys {
            api_keys: Vec<AbsApiKey>,
        }
        let resp = self
            .client
            .get(format!("{}/api/api-keys", self.base_url))
            .bearer_auth(admin_key)
            .send()
            .await?
            .error_for_status()?
            .json::<Keys>()
            .await?;
        Ok(resp.api_keys)
    }

    pub async fn delete_api_key(&self, admin_key: &str, key_id: &str) -> Result<()> {
        self.client
            .delete(format!("{}/api/api-keys/{}", self.base_url, key_id))
            .bearer_auth(admin_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
