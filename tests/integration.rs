//! End-to-end integration tests that exercise the public library API
//! against a sandboxed `HOME` (via `Paths::resolve_under`).
//!
//! These do NOT spawn the binary -- they call into the lib so we can
//! pass a synthetic home directory. The CLI subcommands are tested
//! separately by `cli_smoke.rs` via `assert_cmd`, where the test
//! fixture mutates the env.

use claude_go::paths::Paths;
use claude_go::provider::{CustomProviderEntry, ProviderFormat};
use claude_go::settings::{self, SettingsState, TurnOnInputs};
use claude_go::provider::{self, ModelSource};
use claude_go::proxy::Proxy;
use claude_go::proxy::ProxyState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn fresh_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("claude-go-int-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn test_provider(name: &str, base_url: &str, format: ProviderFormat) -> claude_go::provider::Provider {
    claude_go::provider::Provider {
        id: name.into(),
        display_name: name.into(),
        base_url: base_url.into(),
        format,
        auth_header: "x-api-key".into(),
        model_source: ModelSource::Any,
        implemented: true,
        is_custom: false,
    }
}

#[test]
fn on_writes_correct_settings_json() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let provider = test_provider("opencode-go", "https://opencode.ai/zen/go", ProviderFormat::Anthropic);

    settings::turn_on(
        &paths,
        &TurnOnInputs {
            provider: &provider,
            model: "minimax-m3",
            format: ProviderFormat::Anthropic,
            port: None,
            auth_token: "sk-test",
        },
    )
    .unwrap();

    let raw = std::fs::read_to_string(&paths.settings_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env = v.get("env").and_then(|o| o.as_object()).unwrap();
    // 9 owned keys + 1 marker = 10.
    assert_eq!(env.len(), 10);
    assert_eq!(env["ANTHROPIC_BASE_URL"], "https://opencode.ai/zen/go");
    assert_eq!(env["ANTHROPIC_MODEL"], "minimax-m3");
    assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "minimax-m3");
    assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL"], "minimax-m3");
    assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL"], "minimax-m3");
    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "sk-test");
    assert_eq!(env["ANTHROPIC_API_KEY"], "sk-test");
    assert_eq!(env["DISABLE_TELEMETRY"], "1");
    assert_eq!(env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    assert_eq!(env["__claude_go_owned"], "1");
}

#[test]
fn off_strips_owned_keys_and_marker() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let provider = test_provider("opencode-go", "https://opencode.ai/zen/go", ProviderFormat::Anthropic);

    settings::turn_on(
        &paths,
        &TurnOnInputs {
            provider: &provider,
            model: "minimax-m3",
            format: ProviderFormat::Anthropic,
            port: None,
            auth_token: "sk-test",
        },
    )
    .unwrap();

    settings::turn_off(&paths).unwrap();

    let raw = std::fs::read_to_string(&paths.settings_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env = v.get("env").and_then(|o| o.as_object());
    // env is dropped entirely because no other env vars were set.
    assert!(env.is_none() || env.unwrap().is_empty());

    let state = SettingsState::peek(&paths).unwrap();
    assert!(!state.enabled);
}

#[test]
fn off_preserves_user_owned_env_vars() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let user_value = serde_json::json!({
        "permissions": {"defaultMode": "auto"},
        "env": {
            "MY_PERSONAL_VAR": "do-not-touch",
            "PATH_EXTRA": "/opt/mything/bin"
        }
    });
    std::fs::create_dir_all(paths.settings_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.settings_file,
        serde_json::to_vec_pretty(&user_value).unwrap(),
    )
    .unwrap();

    let provider = test_provider("opencode-go", "https://opencode.ai/zen/go", ProviderFormat::Anthropic);
    settings::turn_on(
        &paths,
        &TurnOnInputs {
            provider: &provider,
            model: "minimax-m3",
            format: ProviderFormat::Anthropic,
            port: None,
            auth_token: "sk-test",
        },
    )
    .unwrap();

    settings::turn_off(&paths).unwrap();

    let raw = std::fs::read_to_string(&paths.settings_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env = v.get("env").and_then(|o| o.as_object()).unwrap();
    assert_eq!(env["MY_PERSONAL_VAR"], "do-not-touch");
    assert_eq!(env["PATH_EXTRA"], "/opt/mything/bin");
    // The marker must be gone after off.
    assert!(env.get("__claude_go_owned").is_none());
    // The 9 owned keys must be gone too.
    assert!(env.get("ANTHROPIC_BASE_URL").is_none());
    assert!(env.get("ANTHROPIC_AUTH_TOKEN").is_none());
}

#[test]
fn off_without_marker_leaves_user_keys_alone() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    // User has their own ANTHROPIC_* setup with no marker.
    let user_value = serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": "https://internal.proxy.corp",
            "ANTHROPIC_AUTH_TOKEN": "user-key",
            "ANTHROPIC_MODEL": "claude-internal"
        }
    });
    std::fs::create_dir_all(paths.settings_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.settings_file,
        serde_json::to_vec_pretty(&user_value).unwrap(),
    )
    .unwrap();

    // off should be a no-op (idempotent, safe).
    settings::turn_off(&paths).unwrap();

    let raw = std::fs::read_to_string(&paths.settings_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env = v.get("env").and_then(|o| o.as_object()).unwrap();
    assert_eq!(env["ANTHROPIC_BASE_URL"], "https://internal.proxy.corp");
    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "user-key");
    assert_eq!(env["ANTHROPIC_MODEL"], "claude-internal");
}

#[test]
fn openai_format_uses_localhost_port_in_base_url() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let provider = claude_go::provider::Provider {
        id: "opencode-go".into(),
        display_name: "OpenCode Go".into(),
        base_url: "https://opencode.ai/zen/go".into(),
        format: ProviderFormat::OpenAI,
        auth_header: "x-api-key".into(),
        model_source: ModelSource::Any,
        implemented: true,
        is_custom: false,
    };

    settings::turn_on(
        &paths,
        &TurnOnInputs {
            provider: &provider,
            model: "glm-5.2",
            format: ProviderFormat::OpenAI,
            port: Some(4188),
            auth_token: "sk-test",
        },
    )
    .unwrap();

    let raw = std::fs::read_to_string(&paths.settings_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let env = v.get("env").and_then(|o| o.as_object()).unwrap();
    assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:4188");
}

#[test]
fn custom_provider_add_remove_round_trip() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let entry = CustomProviderEntry {
        name: "MyCorp".into(),
        base_url: "https://llm.internal.corp".into(),
        format: ProviderFormat::Anthropic,
        auth_header: "x-api-key".into(),
        models: vec!["custom-model-1".into()],
    };
    provider::add_custom_provider(&paths.providers_file, "mycorp", entry).unwrap();

    let custom = provider::load_custom_providers(&paths.providers_file);
    assert!(custom.providers.contains_key("mycorp"));
    let list = provider::provider_list(&custom);
    assert!(list.iter().any(|p| p.id == "mycorp"));

    provider::remove_custom_provider(&paths.providers_file, "mycorp").unwrap();
    let custom2 = provider::load_custom_providers(&paths.providers_file);
    assert!(!custom2.providers.contains_key("mycorp"));
}

#[test]
fn custom_provider_rejects_builtin_id() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let entry = CustomProviderEntry {
        name: "Imposter".into(),
        base_url: "https://nope".into(),
        format: ProviderFormat::Anthropic,
        auth_header: "x-api-key".into(),
        models: vec![],
    };
    // Built-in id "opencode-go" must be rejected.
    let err = provider::add_custom_provider(&paths.providers_file, "opencode-go", entry).unwrap_err();
    match err {
        claude_go::error::ClaudeGoError::ProviderAlreadyExists(_) => {}
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn proxy_lifecycle_start_stop_idempotent() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let proxy = Proxy::new(&paths);

    // Initial state: stopped.
    assert_eq!(proxy.current_state(), ProxyState::Stopped);

    // stop() on a stopped proxy is a no-op.
    assert!(proxy.stop().is_ok());
    assert!(proxy.stop().is_ok());
}

#[test]
fn settings_state_reports_enabled_iff_full_block() {
    let home = fresh_home();
    let paths = Paths::resolve_under(&home);
    let provider = test_provider("opencode-go", "https://opencode.ai/zen/go", ProviderFormat::Anthropic);

    // Before on: not enabled.
    let s0 = SettingsState::peek(&paths).unwrap();
    assert!(!s0.enabled);

    settings::turn_on(
        &paths,
        &TurnOnInputs {
            provider: &provider,
            model: "minimax-m3",
            format: ProviderFormat::Anthropic,
            port: None,
            auth_token: "sk-test",
        },
    )
    .unwrap();
    let s1 = SettingsState::peek(&paths).unwrap();
    assert!(s1.enabled);
    assert_eq!(s1.model, "minimax-m3");
    assert_eq!(s1.path_kind, claude_go::settings::PathKind::Anthropic);
}
