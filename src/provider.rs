use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::ClaudeGoError;

/// Wire format of a provider's endpoint. Determines whether we route
/// directly (`Anthropic`) or need the `opencode-api` translation proxy
/// (`OpenAI`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderFormat {
    /// Provider speaks Anthropic Messages format directly. No proxy.
    Anthropic,
    /// Provider speaks OpenAI Chat Completions. Needs the local proxy.
    OpenAI,
}

impl ProviderFormat {
    /// True iff this provider needs the `opencode-api` translation proxy
    /// to be running for Claude Code to talk to it.
    pub fn needs_proxy(self) -> bool {
        matches!(self, ProviderFormat::OpenAI)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderFormat::Anthropic => "anthropic",
            ProviderFormat::OpenAI => "openai",
        }
    }
}

/// What kind of model list the provider exposes. This is "parse, don't
/// validate": once we know which variant a provider is, we know exactly
/// where the model list comes from and we don't have to re-check later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSource {
    /// A fixed, baked-in list of model ids. Used for the OpenCode Go
    /// fallback list and for presets that only support a small set.
    Fixed(Vec<Model>),
    /// A live HTTP endpoint that returns `{ "data": [{ "id": "..." }] }`.
    /// Fetched on demand; cached in-process for 5 min by the caller.
    Live {
        url: String,
        fallback: Vec<Model>,
    },
    /// The provider accepts any model id (e.g. Anthropic direct, custom
    /// Anthropic-format proxies).
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub description: String,
}

impl Model {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

/// A resolved provider, after parsing the custom-providers file.
///
/// "Resolved" means: we have a single concrete `ProviderFormat`, an auth
/// header name, a base URL, a model source, and a flag for "is this
/// fully implemented or just a stub?". The rest of the code never sees
/// a half-built provider; if we got one, it's either a real preset or
/// a parsed custom entry, and either way the types guarantee its
/// fields are present and well-formed.
#[derive(Debug, Clone)]
pub struct Provider {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub format: ProviderFormat,
    pub auth_header: String,
    pub model_source: ModelSource,
    /// True for presets whose `claude-go on` flow is wired up. False
    /// for stubs (Cloudflare, Vertex, Bedrock) and for "Custom URL..."
    /// (which prompts for input).
    pub implemented: bool,
    /// True for user-supplied providers from `providers.json`. These
    /// can be removed with `claude-go provider remove`.
    pub is_custom: bool,
}

impl Provider {
    pub fn format(&self) -> ProviderFormat {
        self.format
    }
}

/// On-disk shape of `~/.config/claude-go/providers.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CustomProvidersFile {
    #[serde(default)]
    pub providers: BTreeMap<String, CustomProviderEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProviderEntry {
    pub name: String,
    pub base_url: String,
    pub format: ProviderFormat,
    pub auth_header: String,
    #[serde(default)]
    pub models: Vec<String>,
}

/// The hardcoded 19-model OpenCode Go list, taken verbatim from the
/// bash v0.1.1 reference. Used as the fallback when the live
/// `/v1/models` endpoint can't be reached.
fn opencode_go_models() -> Vec<Model> {
    vec![
        // Anthropic Messages -- direct
        Model::new("minimax-m3", "default model; Anthropic Messages direct"),
        Model::new("minimax-m2.7", "Anthropic Messages direct"),
        Model::new("minimax-m2.5", "Anthropic Messages direct"),
        Model::new("qwen3.7-max", "Anthropic Messages direct"),
        Model::new("qwen3.7-plus", "Anthropic Messages direct"),
        Model::new("qwen3.6-plus", "Anthropic Messages direct"),
        // OpenAI Chat Completions -- needs proxy
        Model::new("glm-5.2", "OpenAI Chat Completions via proxy"),
        Model::new("glm-5.1", "OpenAI Chat Completions via proxy"),
        Model::new("glm-5", "OpenAI Chat Completions via proxy"),
        Model::new("kimi-k2.7-code", "OpenAI Chat Completions via proxy"),
        Model::new("kimi-k2.6", "OpenAI Chat Completions via proxy"),
        Model::new("kimi-k2.5", "OpenAI Chat Completions via proxy"),
        Model::new("deepseek-v4-pro", "OpenAI Chat Completions via proxy"),
        Model::new("deepseek-v4-flash", "OpenAI Chat Completions via proxy"),
        Model::new("mimo-v2.5", "OpenAI Chat Completions via proxy"),
        Model::new("mimo-v2.5-pro", "OpenAI Chat Completions via proxy"),
        Model::new("mimo-v2-pro", "OpenAI Chat Completions via proxy"),
        Model::new("mimo-v2-omni", "OpenAI Chat Completions via proxy"),
        Model::new("hy3-preview", "OpenAI Chat Completions via proxy"),
    ]
}

/// All built-in presets, in the order they appear in the TUI. Order is
/// deliberate: working presets first, stubs last.
pub fn built_in_presets() -> Vec<Provider> {
    vec![
        Provider {
            id: "opencode-go".into(),
            display_name: "OpenCode Go".into(),
            base_url: "https://opencode.ai/zen/go".into(),
            format: ProviderFormat::Anthropic,
            // OpenCode Go's anthropic endpoint accepts the standard
            // `x-api-key` header. The value comes from `OPENCODE_API_KEY`
            // (or `ANTHROPIC_AUTH_TOKEN` in settings.json).
            auth_header: "x-api-key".into(),
            model_source: ModelSource::Live {
                url: "https://opencode.ai/zen/go/v1/models".into(),
                fallback: opencode_go_models(),
            },
            implemented: true,
            is_custom: false,
        },
        Provider {
            id: "anthropic-direct".into(),
            display_name: "Anthropic direct".into(),
            base_url: "https://api.anthropic.com".into(),
            format: ProviderFormat::Anthropic,
            auth_header: "x-api-key".into(),
            model_source: ModelSource::Any,
            implemented: true,
            is_custom: false,
        },
        Provider {
            id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            format: ProviderFormat::Anthropic,
            auth_header: "Authorization".into(),
            model_source: ModelSource::Any,
            implemented: true,
            is_custom: false,
        },
        Provider {
            id: "cloudflare".into(),
            display_name: "Cloudflare AI Gateway".into(),
            base_url: "https://gateway.ai.cloudflare.com/v1".into(),
            format: ProviderFormat::Anthropic,
            auth_header: "Authorization".into(),
            model_source: ModelSource::Any,
            implemented: false,
            is_custom: false,
        },
        Provider {
            id: "vertex".into(),
            display_name: "Google Vertex (Claude)".into(),
            base_url: "https://aiplatform.googleapis.com".into(),
            format: ProviderFormat::Anthropic,
            auth_header: "Authorization".into(),
            model_source: ModelSource::Any,
            implemented: false,
            is_custom: false,
        },
        Provider {
            id: "bedrock".into(),
            display_name: "AWS Bedrock (Claude)".into(),
            base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".into(),
            format: ProviderFormat::Anthropic,
            auth_header: "Authorization".into(),
            model_source: ModelSource::Any,
            implemented: false,
            is_custom: false,
        },
    ]
}

/// Load custom providers from `paths.providers_file`. Returns an empty
/// file-shaped struct if the file does not exist or fails to parse as
/// JSON -- we don't want a corrupt providers.json to brick the TUI.
pub fn load_custom_providers(path: &std::path::Path) -> CustomProvidersFile {
    let Ok(bytes) = std::fs::read(path) else {
        return CustomProvidersFile::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

impl CustomProviderEntry {
    /// Convert a parsed custom entry into a runtime `Provider`. The
    /// `implemented` flag is always `true` for custom providers; the
    /// user added them, so they should be selectable.
    pub fn into_provider(self, id: &str) -> Provider {
        let models: Vec<Model> = self
            .models
            .iter()
            .map(|m| Model::new(m.clone(), format!("custom: {m}")))
            .collect();
        let model_source = if models.is_empty() {
            ModelSource::Any
        } else {
            ModelSource::Fixed(models)
        };
        Provider {
            id: id.to_string(),
            display_name: self.name,
            base_url: self.base_url,
            format: self.format,
            auth_header: self.auth_header,
            model_source,
            implemented: true,
            is_custom: true,
        }
    }
}

/// The "Custom URL..." sentinel rendered at the bottom of the provider
/// list. Selecting it in the TUI prompts the user to type a new base
/// URL.
pub const CUSTOM_URL_ID: &str = "__custom_url__";

/// Build the full ordered provider list for the TUI: built-ins first,
/// then custom entries, then the "Custom URL..." sentinel.
pub fn provider_list(custom: &CustomProvidersFile) -> Vec<Provider> {
    let mut all = built_in_presets();
    for (id, entry) in custom.providers.iter() {
        all.push(entry.clone().into_provider(id));
    }
    all.push(Provider {
        id: CUSTOM_URL_ID.into(),
        display_name: "Custom URL...".into(),
        base_url: String::new(),
        format: ProviderFormat::Anthropic,
        auth_header: "x-api-key".into(),
        model_source: ModelSource::Any,
        implemented: true,
        is_custom: false,
    });
    all
}

/// Look up a provider by id from a built-in + custom list.
pub fn find_provider<'a>(list: &'a [Provider], id: &str) -> Option<&'a Provider> {
    list.iter().find(|p| p.id == id)
}

/// Write the custom providers file atomically.
pub fn save_custom_providers(
    path: &std::path::Path,
    file: &CustomProvidersFile,
) -> Result<(), ClaudeGoError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    let bytes = serde_json::to_vec_pretty(file)?;
    use std::io::Write;
    tmp.as_file().write_all(&bytes)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Add a custom provider. Refuses to overwrite an existing id.
pub fn add_custom_provider(
    path: &std::path::Path,
    id: &str,
    entry: CustomProviderEntry,
) -> Result<(), ClaudeGoError> {
    if id == CUSTOM_URL_ID || built_in_presets().iter().any(|p| p.id == id) {
        return Err(ClaudeGoError::ProviderAlreadyExists(id.to_string()));
    }
    let mut file = load_custom_providers(path);
    if file.providers.contains_key(id) {
        return Err(ClaudeGoError::ProviderAlreadyExists(id.to_string()));
    }
    file.providers.insert(id.to_string(), entry);
    save_custom_providers(path, &file)
}

/// Remove a custom provider. Refuses to remove built-in ids.
pub fn remove_custom_provider(
    path: &std::path::Path,
    id: &str,
) -> Result<(), ClaudeGoError> {
    if built_in_presets().iter().any(|p| p.id == id) {
        return Err(ClaudeGoError::ProviderNotFound(id.to_string()));
    }
    let mut file = load_custom_providers(path);
    if file.providers.remove(id).is_none() {
        return Err(ClaudeGoError::ProviderNotFound(id.to_string()));
    }
    save_custom_providers(path, &file)
}

/// True iff the model id is one of the OpenCode Go OpenAI-format
/// models (i.e. needs the local `opencode-api` proxy). This is the
/// single source of truth for "should the proxy be running for this
/// model?" and is the same hardcoded list the bash v0.1.1 reference
/// uses.
pub fn is_opencode_go_openai_model(model: &str) -> bool {
    matches!(
        model,
        "glm-5.2"
            | "glm-5.1"
            | "glm-5"
            | "kimi-k2.7-code"
            | "kimi-k2.6"
            | "kimi-k2.5"
            | "deepseek-v4-pro"
            | "deepseek-v4-flash"
            | "mimo-v2.5"
            | "mimo-v2.5-pro"
            | "mimo-v2-pro"
            | "mimo-v2-omni"
            | "hy3-preview"
    )
}
