use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::error::ClaudeGoError;
use crate::provider::{Model, ModelSource};

/// How long the live model-list response is cached in-process.
/// 5 minutes, matching the spec.
const LIVE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
/// Timeout for the live fetch. 5s, fail-fast.
const LIVE_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Process-wide cache of the OpenCode Go live model list. Storing the
/// `Option<Vec<Model>>` lets us cache both the "fetched successfully"
/// case and the "fetch failed, using fallback" case without re-trying
/// on every screen render.
static OPENCODE_GO_CACHE: Lazy<Mutex<Option<CacheEntry>>> = Lazy::new(|| Mutex::new(None));

struct CacheEntry {
    fetched_at: Instant,
    models: Vec<Model>,
    /// True if the live fetch succeeded; false if we used the fallback
    /// because the network call failed.
    from_live: bool,
}

/// Return the OpenCode Go model list. Tries the live endpoint first;
/// on failure (or cache hit), returns the cached or fallback list.
pub async fn opencode_go_models() -> (Vec<Model>, bool) {
    // Cache hit?
    if let Some(entry) = OPENCODE_GO_CACHE.lock().unwrap().as_ref() {
        if entry.fetched_at.elapsed() < LIVE_CACHE_TTL {
            return (entry.models.clone(), entry.from_live);
        }
    }

    let url = "https://opencode.ai/zen/go/v1/models";
    let client = match reqwest::Client::builder()
        .timeout(LIVE_FETCH_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return (fallback(), false),
    };

    let resp = client.get(url).send().await;
    let models = match resp {
        Ok(r) if r.status().is_success() => match r.json::<OpenCodeGoList>().await {
            Ok(list) => list
                .data
                .into_iter()
                .map(|m| {
                    let id = m.id.clone();
                    Model::new(m.id, format!("live: {id}"))
                })
                .collect(),
            Err(_) => fallback(),
        },
        _ => fallback(),
    };
    let from_live = !models.is_empty() && models[0].description.starts_with("live: ");

    *OPENCODE_GO_CACHE.lock().unwrap() = Some(CacheEntry {
        fetched_at: Instant::now(),
        models: models.clone(),
        from_live,
    });

    (models, from_live)
}

#[derive(serde::Deserialize)]
struct OpenCodeGoList {
    data: Vec<OpenCodeGoEntry>,
}

#[derive(serde::Deserialize)]
struct OpenCodeGoEntry {
    id: String,
}

/// Synchronous fallback for contexts where we can't await (e.g. the
/// `models` CLI subcommand). Returns the hardcoded 19-model list.
pub fn fallback() -> Vec<Model> {
    // Built-in OpenCode Go provider is at index 0 in `built_in_presets()`.
    let preset = &crate::provider::built_in_presets()[0];
    match &preset.model_source {
        ModelSource::Live { fallback, .. } => fallback.clone(),
        _ => unreachable!("opencode-go preset must have a Live model source"),
    }
}

/// Validate a model id against a provider's model source. For `Any`,
/// anything non-empty is acceptable. For `Fixed`, the id must be in
/// the list. For `Live`, the id must be in either the live cache or
/// the fallback.
pub fn validate_model(source: &ModelSource, id: &str) -> Result<(), ClaudeGoError> {
    if id.is_empty() {
        return Err(ClaudeGoError::UnknownModel(id.to_string()));
    }
    match source {
        ModelSource::Any => Ok(()),
        ModelSource::Fixed(list) => {
            if list.iter().any(|m| m.id == id) {
                Ok(())
            } else {
                Err(ClaudeGoError::UnknownModel(id.to_string()))
            }
        }
        ModelSource::Live { fallback, .. } => {
            // Best-effort: also accept the live cache.
            if let Some(entry) = OPENCODE_GO_CACHE.lock().unwrap().as_ref() {
                if entry.models.iter().any(|m| m.id == id) {
                    return Ok(());
                }
            }
            if fallback.iter().any(|m| m.id == id) {
                Ok(())
            } else {
                Err(ClaudeGoError::UnknownModel(id.to_string()))
            }
        }
    }
}
