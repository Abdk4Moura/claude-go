use thiserror::Error;

/// Errors that can occur in the claude-go library.
///
/// This is a typed error type for cases where the caller benefits from
/// pattern-matching (e.g. CLI subcommands distinguishing "settings not
/// configured" from "I/O failure"). Anything that does not benefit from
/// a typed variant is bubbled up as `anyhow::Error` instead.
#[derive(Debug, Error)]
pub enum ClaudeGoError {
    #[error("settings file is not valid JSON: {0}")]
    InvalidSettingsJson(#[from] serde_json::Error),

    #[error("settings.json `env` must be an object, got {found}. Edit it manually first.")]
    EnvNotObject { found: String },

    #[error("OPENCODE_API_KEY is not set. Get one at https://opencode.ai/auth and re-run.")]
    MissingApiKey,

    #[error("unknown model: {0}. Run: claude-go models")]
    UnknownModel(String),

    #[error("invalid port: {0}")]
    InvalidPort(u64),

    #[error("opencode-api binary not found on PATH. Install with: npm install -g opencode-api@1.0.3")]
    ProxyBinaryMissing,

    #[error("proxy failed to start on port {port}; see log at {log}")]
    ProxyStartFailed { port: u16, log: String },

    #[error("proxy is not running (no PID at {0})")]
    ProxyNotRunning(String),

    #[error("provider `{0}` is not yet implemented")]
    ProviderNotImplemented(String),

    #[error("no free port in 4141..=4242")]
    NoFreePort,

    #[error("custom provider `{0}` already exists in providers.json")]
    ProviderAlreadyExists(String),

    #[error("custom provider `{0}` not found in providers.json")]
    ProviderNotFound(String),

    #[error("provider `{0}` has no model list configured and no live fetch URL")]
    NoModelsAvailable(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
