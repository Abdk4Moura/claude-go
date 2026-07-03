use serde::{Deserialize, Serialize};

use crate::error::ClaudeGoError;
use crate::paths::Paths;
use crate::settings::SettingsState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifyOutcome {
    /// 200 OK -- the upstream actually answered with model output.
    Authenticated,
    /// 401 Unauthorized -- round-trip succeeded, auth was rejected.
    /// This is the bash v0.1.1 contract: with a fake/expired key,
    /// 401 means the routing path works, the auth is the failure.
    AuthRejected,
    /// 404 -- endpoint path wrong.
    NotFound,
    /// Connection refused, timeout, DNS error, etc.
    Unreachable,
    /// Any other HTTP status.
    Unexpected(u16),
}

impl VerifyOutcome {
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Authenticated | Self::AuthRejected)
    }

    pub fn message(self) -> String {
        match self {
            Self::Authenticated => "Endpoint reachable, model responded".into(),
            Self::AuthRejected => "Round-trip OK (auth rejected as expected with fake key)".into(),
            Self::NotFound => "Endpoint not found (HTTP 404). Is the base URL correct?".into(),
            Self::Unreachable => "Could not reach the endpoint (network error or timeout)".into(),
            Self::Unexpected(code) => format!("Unexpected HTTP {code}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub outcome: VerifyOutcome,
    pub model: String,
    pub base_url: String,
    pub http_code: u16,
}

/// Run a live verify round-trip. Reads auth from `OPENCODE_API_KEY`
/// (preferred) or from `ANTHROPIC_AUTH_TOKEN` in `settings.json`
/// (fallback, matching bash v0.1.1).
pub async fn verify(paths: &Paths) -> Result<VerifyResult, ClaudeGoError> {
    let state = SettingsState::peek(paths)?;
    if !state.enabled {
        return Err(ClaudeGoError::MissingApiKey);
    }

    let key = std::env::var("OPENCODE_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            // Fallback: read ANTHROPIC_AUTH_TOKEN directly from the
            // settings file. We don't trust our own parsed state for
            // the secret value; we re-read the raw file.
            let Ok(raw) = std::fs::read_to_string(&paths.settings_file) else {
                return None;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
                return None;
            };
            v.get("env")
                .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        });

    let key = match key {
        Some(k) if !k.is_empty() => k,
        _ => return Err(ClaudeGoError::MissingApiKey),
    };

    let url = format!("{}/v1/messages", state.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": state.model,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "ping"}]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ClaudeGoError::Io(std::io::Error::other(e.to_string())))?;

    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return Ok(VerifyResult {
                outcome: VerifyOutcome::Unreachable,
                model: state.model,
                base_url: state.base_url,
                http_code: 0,
            });
        }
    };

    let http_code = response.status().as_u16();
    let outcome = match http_code {
        200 => VerifyOutcome::Authenticated,
        401 => VerifyOutcome::AuthRejected,
        404 => VerifyOutcome::NotFound,
        other => VerifyOutcome::Unexpected(other),
    };

    Ok(VerifyResult {
        outcome,
        model: state.model,
        base_url: state.base_url,
        http_code,
    })
}
