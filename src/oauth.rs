//! Device-code logins.
//!
//! None of these vendors accept a plain browser redirect from a terminal app, so
//! all three use the OAuth device grant: the agent asks for a code, the user
//! approves it on the vendor's own page, and the agent polls until a token comes
//! back. No callback server, no port, nothing to paste.
//!
//! Flows verified against PI's `packages/ai/src/auth/oauth/`.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// GitHub's device-flow client id, read from PI's base64 form.
const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const META_CLIENT_ID: &str = "1031625952748946";

const COPILOT_UA: &str = "GitHubCopilotChat/0.35.0";

/// What the user has to do, shown in the TUI while polling.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub verification_uri: String,
    pub user_code: String,
    /// Seconds between polls, as the vendor asked.
    pub interval: u64,
    pub expires_in: u64,
}

/// The in-flight state each provider needs across polls.
#[derive(Debug, Clone)]
pub enum Flow {
    OpenAiCodex {
        device_auth_id: String,
        user_code: String,
    },
    GitHubCopilot {
        device_code: String,
    },
    Meta {
        device_code: String,
    },
}

pub enum Poll {
    Pending,
    SlowDown,
    /// A credential worth storing, under the key the caller names.
    Ready(String),
}

fn client() -> reqwest::Client {
    // A User-Agent is not optional: Meta's edge serves a login page instead of
    // JSON when the header is missing, and reqwest sets none by default.
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(format!("exp-1-agent/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Starts the device flow for a provider id, returning what to show the user.
pub async fn begin(provider: &str) -> Result<(Prompt, Flow)> {
    match provider {
        "openai-auth" => begin_openai().await,
        "github-copilot" => begin_copilot().await,
        "meta-auth" => begin_meta().await,
        other => bail!("{other} does not use a device login"),
    }
}

/// One poll. The caller sleeps `prompt.interval` between calls.
pub async fn poll(provider: &str, flow: &Flow) -> Result<Poll> {
    match flow {
        Flow::OpenAiCodex {
            device_auth_id,
            user_code,
        } if provider == "openai-auth" => poll_openai(device_auth_id, user_code).await,
        Flow::GitHubCopilot { device_code } if provider == "github-copilot" => {
            poll_copilot(device_code).await
        }
        Flow::Meta { device_code } => poll_meta(device_code).await,
        _ => bail!("the device flow does not match provider {provider}"),
    }
}

// ------------------------------------------------------------------ OpenAI

async fn begin_openai() -> Result<(Prompt, Flow)> {
    let response = client()
        .post("https://auth.openai.com/api/accounts/deviceauth/usercode")
        .json(&json!({ "client_id": OPENAI_CLIENT_ID }))
        .send()
        .await
        .context("ask OpenAI for a device code")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("OpenAI refused the device code: HTTP {status}: {body}");
    }
    let value: Value = response.json().await.context("read the device code")?;
    let device_auth_id = value
        .get("device_auth_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let user_code = value
        .get("user_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if device_auth_id.is_empty() || user_code.is_empty() {
        bail!("OpenAI returned an unusable device code: {value}");
    }
    let interval = value.get("interval").and_then(Value::as_u64).unwrap_or(5);
    Ok((
        Prompt {
            verification_uri: "https://auth.openai.com/codex/device".to_string(),
            user_code,
            interval,
            expires_in: 900,
        },
        Flow::OpenAiCodex {
            device_auth_id,
            user_code: value
                .get("user_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
    ))
}

async fn poll_openai(device_auth_id: &str, user_code: &str) -> Result<Poll> {
    let response = client()
        .post("https://auth.openai.com/api/accounts/deviceauth/token")
        .json(&json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .send()
        .await
        .context("poll the OpenAI device code")?;

    if response.status() == 403 || response.status() == 404 {
        return Ok(Poll::Pending);
    }
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("OpenAI device poll failed: HTTP {status}: {body}");
    }

    // The server hands back its own PKCE pair to exchange for tokens.
    let value: Value = response.json().await.context("read the device token")?;
    let code = value
        .get("authorization_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let verifier = value
        .get("code_verifier")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if code.is_empty() || verifier.is_empty() {
        bail!("OpenAI returned an unusable authorization: {value}");
    }

    let response = client()
        .post("https://auth.openai.com/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("client_id", OPENAI_CLIENT_ID),
            ("redirect_uri", "https://auth.openai.com/deviceauth/callback"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .context("exchange the OpenAI authorization code")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("OpenAI token exchange failed: HTTP {status}: {body}");
    }
    let tokens: Value = response.json().await.context("read the OpenAI tokens")?;
    let refresh = tokens
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let access = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id_token = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if access.is_empty() {
        bail!("OpenAI returned no access token: {tokens}");
    }
    // Store it in the same shape the Codex CLI uses, so the existing reader and
    // refresher keep working untouched.
    let account_id = account_id_from_token(id_token)
        .or_else(|| account_id_from_token(access))
        .unwrap_or_default();
    let stored = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": id_token,
            "access_token": access,
            "refresh_token": refresh,
            "account_id": account_id,
        }
    });
    Ok(Poll::Ready(stored.to_string()))
}

/// Reads `chatgpt_account_id` out of a JWT without verifying it.
fn account_id_from_token(jwt: &str) -> Option<String> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(str::to_string)
}

// ------------------------------------------------------------ GitHub Copilot

async fn begin_copilot() -> Result<(Prompt, Flow)> {
    let response = client()
        .post("https://github.com/login/device/code")
        .header("accept", "application/json")
        .header("user-agent", COPILOT_UA)
        .form(&[("client_id", COPILOT_CLIENT_ID), ("scope", "read:user")])
        .send()
        .await
        .context("ask GitHub for a device code")?;
    let value: Value = response.json().await.context("read the device code")?;
    let device_code = value
        .get("device_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let user_code = value
        .get("user_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let verification_uri = value
        .get("verification_uri")
        .and_then(Value::as_str)
        .unwrap_or("https://github.com/login/device")
        .to_string();
    if device_code.is_empty() || user_code.is_empty() {
        bail!("GitHub returned an unusable device code: {value}");
    }
    Ok((
        Prompt {
            verification_uri,
            user_code,
            interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
            expires_in: value.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
        },
        Flow::GitHubCopilot { device_code },
    ))
}

async fn poll_copilot(device_code: &str) -> Result<Poll> {
    let response = client()
        .post("https://github.com/login/oauth/access_token")
        .header("accept", "application/json")
        .header("user-agent", COPILOT_UA)
        .form(&[
            ("client_id", COPILOT_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .context("poll the GitHub device code")?;
    let value: Value = response.json().await.context("read the GitHub token")?;

    if let Some(token) = value.get("access_token").and_then(Value::as_str) {
        // The GitHub token is long-lived; the Copilot token it buys is not, so
        // the long-lived one is what gets stored.
        return Ok(Poll::Ready(token.to_string()));
    }
    match value.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown),
        Some(other) => bail!("GitHub login failed: {other}"),
        None => bail!("GitHub returned neither a token nor an error: {value}"),
    }
}

/// Exchanges a stored GitHub token for a Copilot bearer token and the API host
/// it belongs to. The token carries `proxy-ep=<host>`.
pub async fn copilot_bearer(github_token: &str) -> Result<(String, String)> {
    let response = client()
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("accept", "application/json")
        .header("user-agent", COPILOT_UA)
        .header("editor-version", "vscode/1.107.0")
        .header("editor-plugin-version", "copilot-chat/0.35.0")
        .header("copilot-integration-id", "vscode-chat")
        .bearer_auth(github_token)
        .send()
        .await
        .context("exchange the GitHub token for a Copilot token")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Copilot token exchange failed: HTTP {status}: {body}");
    }
    let value: Value = response.json().await.context("read the Copilot token")?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        bail!("Copilot returned no token: {value}");
    }
    // `proxy-ep=proxy.individual.githubcopilot.com` -> api.individual.githubcopilot.com
    let host = token
        .split(';')
        .find_map(|part| part.strip_prefix("proxy-ep="))
        .map(|host| host.replace("proxy.", "api."))
        .unwrap_or_else(|| "api.individual.githubcopilot.com".to_string());
    Ok((format!("https://{host}"), token))
}

// -------------------------------------------------------------------- Meta

async fn begin_meta() -> Result<(Prompt, Flow)> {
    let response = client()
        .post("https://auth.meta.com/oidc/device/authorization/")
        .header("accept", "application/json")
        .form(&[("client_id", META_CLIENT_ID)])
        .send()
        .await
        .context("ask Meta for a device code")?;
    let status = response.status();
    let body = response.text().await.context("read the Meta response")?;
    let value: Value = serde_json::from_str(&body).with_context(|| {
        format!("Meta device authorization returned HTTP {status}: {}", &body[..body.len().min(200)])
    })?;
    let device_code = value
        .get("device_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let user_code = value
        .get("user_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let verification_uri = value
        .get("verification_uri_complete")
        .and_then(Value::as_str)
        .or_else(|| value.get("verification_uri").and_then(Value::as_str))
        .unwrap_or("https://auth.meta.com/device")
        .to_string();
    if device_code.is_empty() || user_code.is_empty() {
        bail!("Meta returned an unusable device code: {value}");
    }
    Ok((
        Prompt {
            verification_uri,
            user_code,
            interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
            expires_in: value.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
        },
        Flow::Meta { device_code },
    ))
}

async fn poll_meta(device_code: &str) -> Result<Poll> {
    let response = client()
        .post("https://auth.meta.com/oidc/device/token/")
        .header("accept", "application/json")
        .form(&[
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code",
            ),
            ("device_code", device_code),
            ("client_id", META_CLIENT_ID),
        ])
        .send()
        .await
        .context("poll the Meta device code")?;
    let value: Value = response.json().await.context("read the Meta token")?;

    let Some(identity) = value.get("access_token").and_then(Value::as_str) else {
        return match value.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => Ok(Poll::Pending),
            Some("slow_down") => Ok(Poll::SlowDown),
            Some(other) => bail!("Meta login failed: {other}"),
            None => bail!("Meta returned neither a token nor an error: {value}"),
        };
    };

    // The identity token buys a Model API key, which is what requests use.
    let response = client()
        .post("https://api.meta.ai/muse-code/key")
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("x-api-version", "1.0.0")
        .bearer_auth(identity)
        .body("{}")
        .send()
        .await
        .context("mint a Meta Model API key")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Meta key minting failed: HTTP {status}: {body}");
    }
    let value: Value = response.json().await.context("read the Meta key")?;
    let key = ["key", "api_key", "apiKey"]
        .iter()
        .find_map(|field| value.get(field).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    if key.is_empty() {
        bail!("Meta returned no API key: {value}");
    }
    Ok(Poll::Ready(key))
}

// ------------------------------------------------------------------ Vertex

/// Where `gcloud auth application-default login` puts its credentials.
fn adc_path() -> std::path::PathBuf {
    std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join(".config/gcloud/application_default_credentials.json")
        })
}

/// The Vertex configuration, read from the environment the way
/// `gcloud auth application-default login` leaves it.
#[derive(Debug, Clone)]
pub struct VertexConfig {
    pub project: String,
    pub location: String,
    pub access_token: String,
}

/// Turns Application Default Credentials into an access token.
///
/// Only the `authorized_user` shape that gcloud writes is handled; a service
/// account file would need an RS256-signed JWT, which is a different job.
pub async fn vertex_config() -> Result<VertexConfig> {
    let project = std::env::var("GOOGLE_CLOUD_PROJECT")
        .or_else(|_| std::env::var("GCLOUD_PROJECT"))
        .map_err(|_| anyhow::anyhow!("GOOGLE_CLOUD_PROJECT is not set"))?;
    let location = std::env::var("GOOGLE_CLOUD_LOCATION")
        .map_err(|_| anyhow::anyhow!("GOOGLE_CLOUD_LOCATION is not set"))?;

    let path = adc_path();
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "no Google credentials at {} — run `gcloud auth application-default login`",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&raw).context("parse the Google credentials file")?;

    if value.get("type").and_then(Value::as_str) != Some("authorized_user") {
        bail!(
            "{} is not an authorized_user file; service-account keys need a signed JWT, which \
             this agent does not mint",
            path.display()
        );
    }
    let client_id = value
        .get("client_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("credentials file has no client_id"))?;
    let client_secret = value
        .get("client_secret")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("credentials file has no client_secret"))?;
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("credentials file has no refresh_token"))?;

    let response = client()
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("exchange the Google refresh token")?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Google token exchange failed: HTTP {status}: {body}");
    }
    let tokens: Value = response.json().await.context("read the Google token")?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Google returned no access token: {tokens}"))?
        .to_string();

    Ok(VertexConfig {
        project,
        location,
        access_token,
    })
}

/// Opens the vendor's sign-in page in the user's browser, the way `codex login`
/// does. Best effort by design: the URL is always printed as well, so a machine
/// with no browser loses nothing by this failing.
pub fn open_in_browser(url: &str) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ------------------------------------------------------------- GitLab Duo
/// Exchanges a GitLab personal access token for the OIDC token the Duo
/// gateway insists on: a plain `glpat-` is rejected with
/// "only OIDC is supported".
pub async fn gitlab_oidc(pat: &str, instance: &str) -> Result<String> {
    let url = format!(
        "{}/api/v4/ai/duo_workflows/direct_access",
        instance.trim_end_matches('/')
    );
    let response = client()
        .post(&url)
        .header("content-type", "application/json")
        .bearer_auth(pat)
        .json(&json!({ "workflow_definition": "chat" }))
        .send()
        .await
        .context("ask GitLab for a Duo token")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        // 403 carries GitLab's own explanation of what the account is missing.
        bail!("GitLab Duo refused the token (HTTP {status}): {}", body.chars().take(300).collect::<String>());
    }
    // The body is the token itself, as a JSON string.
    let token = serde_json::from_str::<String>(&body).unwrap_or(body);
    if token.trim().is_empty() {
        bail!("GitLab Duo returned an empty token");
    }
    Ok(token.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_account_id_out_of_a_jwt() {
        use base64::Engine;
        let claims = json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct-123" }
        });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).expect("encode"));
        let jwt = format!("header.{payload}.signature");
        assert_eq!(account_id_from_token(&jwt), Some("acct-123".to_string()));
        assert_eq!(account_id_from_token("not-a-jwt"), None);
    }

    #[test]
    fn reads_the_adc_path_from_the_environment() {
        // The override exists so a service-account file can be pointed at.
        unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", "/tmp/creds.json") };
        assert_eq!(adc_path().to_string_lossy(), "/tmp/creds.json");
        unsafe { std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS") };
    }

    #[test]
    fn derives_the_copilot_host_from_the_proxy_hint() {
        let token = "tid=abc;exp=123;proxy-ep=proxy.individual.githubcopilot.com;st=dotcom";
        let host = token
            .split(';')
            .find_map(|part| part.strip_prefix("proxy-ep="))
            .map(|host| host.replace("proxy.", "api."))
            .unwrap_or_default();
        assert_eq!(host, "api.individual.githubcopilot.com");
    }
}