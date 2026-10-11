//! Codex (ChatGPT OAuth) authentication.
//!
//! Credentials live in `$CODEX_HOME/auth.json` (default `~/.codex/auth.json`).
//! That file belongs to the real Codex CLI, so we never rewrite it: on first run
//! it is copied verbatim into our own state file, and only that copy is mutated
//! when a refresh rotates the tokens.

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Refresh this many seconds before the access token actually expires.
const REFRESH_SKEW_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    /// Raw JWT; kept verbatim so it can be written back unchanged.
    pub id_token: String,
    /// Raw JWT.
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub struct Auth {
    state_path: PathBuf,
    file: AuthFile,
}

/// Reads the `exp` claim out of a JWT without verifying the signature.
fn jwt_exp(jwt: &str) -> Option<i64> {
    let mut parts = jwt.split('.');
    let (_header, payload) = (parts.next()?, parts.next()?);
    parts.next()?; // signature must exist for this to be a real JWT
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp")?.as_i64()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Auth {
    /// Our mutable copy of the credentials.
    pub fn state_path() -> PathBuf {
        home().join(".exp1-agent").join("auth.json")
    }

    /// The read-only Codex CLI credential file we seed from.
    pub fn codex_path() -> PathBuf {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".codex"))
            .join("auth.json")
    }

    pub fn load() -> Result<Self> {
        let state_path = Self::state_path();

        let raw = match fs::read_to_string(&state_path) {
            Ok(raw) => raw,
            Err(_) => {
                let source = Self::codex_path();
                let raw = fs::read_to_string(&source).with_context(|| {
                    format!(
                        "no credentials found at {} — sign in with the Codex CLI first, or set CODEX_HOME",
                        source.display()
                    )
                })?;
                if let Some(parent) = state_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&state_path, &raw)
                    .with_context(|| format!("write {}", state_path.display()))?;
                raw
            }
        };

        let file: AuthFile = serde_json::from_str(&raw).context("parse auth.json")?;
        if file.tokens.is_none() {
            bail!(
                "{} contains no ChatGPT tokens — exp-1 agent only supports the Codex auth provider",
                state_path.display()
            );
        }
        Ok(Self { state_path, file })
    }

    pub fn tokens(&self) -> Result<&Tokens> {
        self.file
            .tokens
            .as_ref()
            .context("auth.json has no tokens object")
    }

    pub fn account_id(&self) -> Result<&str> {
        self.tokens()?
            .account_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .context("auth.json has no chatgpt account_id — cannot route Codex requests")
    }

    /// Plan label from `~/.codex/auth.json`, used for the status bar.
    pub fn plan(&self) -> Option<String> {
        let jwt = &self.tokens().ok()?.id_token;
        let payload = jwt.split('.').nth(1)?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value
            .get("https://api.openai.com/auth")?
            .get("chatgpt_plan_type")?
            .as_str()
            .map(str::to_string)
    }

    /// Returns a valid access token, refreshing first when it is close to expiry.
    pub async fn access_token(&mut self) -> Result<String> {
        if self.needs_refresh() {
            self.refresh().await?;
        }
        Ok(self.tokens()?.access_token.clone())
    }

    fn needs_refresh(&self) -> bool {
        let Ok(tokens) = self.tokens() else {
            return true;
        };
        match jwt_exp(&tokens.access_token) {
            Some(exp) => now_unix() + REFRESH_SKEW_SECS >= exp,
            // Unparseable token: try a refresh rather than fail the turn.
            None => true,
        }
    }

    async fn refresh(&mut self) -> Result<()> {
        let current = self.tokens()?.clone();

        let response = reqwest::Client::new()
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", &current.refresh_token),
            ])
            .send()
            .await
            .context("token refresh request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("token refresh rejected ({status}): {body}");
        }

        let fresh: TokenResponse = response.json().await.context("parse token response")?;

        let tokens = self.tokens_mut();
        tokens.access_token = fresh.access_token;
        // The server rotates the refresh token; keeping the old one breaks the next refresh.
        if let Some(rt) = fresh.refresh_token {
            tokens.refresh_token = rt;
        }
        if let Some(it) = fresh.id_token {
            tokens.id_token = it;
        }

        self.persist()
    }

    fn tokens_mut(&mut self) -> &mut Tokens {
        self.file
            .tokens
            .as_mut()
            .expect("tokens presence checked in load()")
    }

    fn persist(&mut self) -> Result<()> {
        self.file.last_refresh = Some(iso_now());
        let json = serde_json::to_vec_pretty(&self.file)?;
        let tmp = self.state_path.with_extension("json.tmp");
        fs::write(&tmp, &json).with_context(|| format!("write {}", tmp.display()))?;
        set_owner_only(&tmp)?;
        fs::rename(&tmp, &self.state_path)
            .with_context(|| format!("replace {}", self.state_path.display()))?;
        Ok(())
    }
}

fn set_owner_only(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn iso_now() -> String {
    let secs = now_unix();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Days since 1970-01-01 -> civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}