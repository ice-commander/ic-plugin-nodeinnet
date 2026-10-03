use nodeinnet_protocol::{RefreshResponse, TurnRegion};
use reqwest::Client;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Refused(u16, String),
    Unanswered(String),
}

pub fn refused(status: u16, body: Option<&serde_json::Value>, fallback: &str) -> String {
    match body
        .and_then(|held| held.get("msg"))
        .and_then(|held| held.as_str())
        .map(str::trim)
        .filter(|said| !said.is_empty())
    {
        Some(said) => said.to_string(),
        None => format!("{fallback} ({status})"),
    }
}

async fn said(resp: reqwest::Response, fallback: &str) -> String {
    let status = resp.status().as_u16();
    let body = resp.json::<serde_json::Value>().await.ok();
    refused(status, body.as_ref(), fallback)
}

fn client() -> Client {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub async fn refresh_access_token(
    api_target: &str,
    refresh_token: &str,
    region: TurnRegion,
) -> Result<RefreshResponse, Failure> {
    let client = client();
    match client
        .post(format!("{}/account/refresh_token", api_target))
        .json(&nodeinnet_protocol::RefreshRequest {
            refresh_token: refresh_token.to_string(),
            region,
        })
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                match serde_json::from_str::<RefreshResponse>(&text) {
                    Ok(login_resp) => Ok(login_resp),
                    Err(e) => Err(Failure::Unanswered(format!(
                        "Failed to parse response: {}",
                        e
                    ))),
                }
            } else {
                let status = resp.status().as_u16();
                Err(Failure::Refused(
                    status,
                    said(resp, "Server returned error status").await,
                ))
            }
        }
        Err(e) => Err(Failure::Unanswered(e.to_string())),
    }
}

pub async fn login(
    api_target: &str,
    login: &str,
    password: &str,
    region: TurnRegion,
) -> Result<nodeinnet_protocol::LoginResponse, Failure> {
    let client = client();
    let resp = client
        .post(format!("{}/account/login", api_target))
        .json(&nodeinnet_protocol::LoginRequest {
            login: login.to_string(),
            password: password.to_string(),
            region,
        })
        .send()
        .await
        .map_err(|e| Failure::Unanswered(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(Failure::Refused(
            status,
            said(resp, "Invalid login or password").await,
        ));
    }
    resp.json::<nodeinnet_protocol::LoginResponse>()
        .await
        .map_err(|e| Failure::Unanswered(format!("Failed to parse login response: {}", e)))
}

pub async fn logoff(
    api_target: &str,
    refresh_token: &str,
    access_token: Option<&str>,
) -> Result<(), Failure> {
    let mut cookie = format!("RefreshToken={refresh_token}");
    if let Some(at) = access_token {
        cookie.push_str("; AccessToken=");
        cookie.push_str(at);
    }
    let resp = client()
        .post(format!("{}/account/logoff", api_target))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .map_err(|e| Failure::Unanswered(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(Failure::Refused(status, said(resp, "logoff failed").await));
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceProfile {
    pub display_name: Option<String>,
    pub os: Option<String>,
    pub app_type: Option<String>,
    pub version: Option<String>,
    pub resources: Vec<nodeinnet_protocol::SharedResource>,
}

pub async fn register_device(
    api_target: &str,
    access_token: &str,
    node_id: &str,
    profile: &DeviceProfile,
) -> Result<nodeinnet_protocol::Device, String> {
    let profile = DeviceProfile {
        resources: profile
            .resources
            .iter()
            .map(|r| r.without_config())
            .collect(),
        ..profile.clone()
    };
    let bson_bytes = bson::serialize_to_vec(&profile)
        .map_err(|e| format!("Failed to encode device profile: {}", e))?;
    let client = client();
    let resp = client
        .post(format!("{}/account/devices", api_target))
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "name": node_id, "resources": bson_bytes }))
        .send()
        .await
        .map_err(|e| format!("Connection error: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Device registration failed ({})", resp.status()));
    }
    resp.json::<nodeinnet_protocol::Device>()
        .await
        .map_err(|e| format!("Failed to parse device response: {}", e))
}
