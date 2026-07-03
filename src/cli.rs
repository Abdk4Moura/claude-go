use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::paths::Paths;
use crate::provider::{
    self, CustomProviderEntry, ProviderFormat, CUSTOM_URL_ID,
};
use crate::proxy::Proxy;
use crate::settings::{self, SettingsState, TurnOnInputs};
use crate::verify;

/// claude-go: route Claude Code to any Anthropic-compatible model.
#[derive(Debug, Parser)]
#[command(name = "claude-go", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Enable routing. Writes ~/.claude/settings.json and starts the proxy if needed.
    On {
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Disable routing. Strips the owned env block, stops the proxy.
    Off,
    /// Show the current state.
    Status,
    /// Round-trip test against the live endpoint.
    Verify,
    /// List known models.
    Models,
    /// List configured providers.
    Providers,
    /// Provider registry management.
    Provider {
        #[command(subcommand)]
        cmd: ProviderCmd,
    },
    /// Install the current binary to ~/.local/bin.
    Install,
    /// Print help.
    Help,
    /// Print version.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ProviderCmd {
    /// Add a custom provider.
    Add {
        name: String,
        #[arg(long)]
        url: String,
        #[arg(long, default_value = "anthropic")]
        format: String,
        #[arg(long, default_value = "x-api-key")]
        auth_header: String,
        #[arg(long)]
        models: Vec<String>,
    },
    /// Remove a custom provider.
    Remove { name: String },
}

/// Run a CLI command. Returns the exit code (0 = success, non-zero =
/// user-facing error).
pub fn run(cli: Cli) -> Result<i32> {
    let paths = Paths::resolve();
    let cmd = match cli.cmd {
        Some(c) => c,
        None => {
            // No-arg = TUI (breaking change vs bash v0.1.1; called
            // out in the README rename banner).
            return run_tui(&paths);
        }
    };
    match cmd {
        Cmd::Help => {
            print_help();
            Ok(0)
        }
        Cmd::Version => {
            println!("claude-go {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Cmd::On { model, port } => cmd_on(&paths, model, port),
        Cmd::Off => cmd_off(&paths),
        Cmd::Status => cmd_status(&paths),
        Cmd::Verify => cmd_verify(&paths),
        Cmd::Models => cmd_models(),
        Cmd::Providers => cmd_providers(&paths),
        Cmd::Provider { cmd } => match cmd {
            ProviderCmd::Add {
                name,
                url,
                format,
                auth_header,
                models,
            } => cmd_provider_add(&paths, name, url, format, auth_header, models),
            ProviderCmd::Remove { name } => cmd_provider_remove(&paths, name),
        },
        Cmd::Install => cmd_install(&paths),
    }
}

fn run_tui(paths: &Paths) -> Result<i32> {
    let app = crate::tui::App::new(paths.clone());
    crate::tui::run(app)?;
    Ok(0)
}

fn cmd_on(paths: &Paths, model: Option<String>, port: Option<u16>) -> Result<i32> {
    let auth = std::env::var("OPENCODE_API_KEY").unwrap_or_default();
    if auth.is_empty() {
        eprintln!("error: OPENCODE_API_KEY is not set. Get one at https://opencode.ai/auth and re-run.");
        return Ok(1);
    }

    let model = model.unwrap_or_else(|| "minimax-m3".into());
    let is_openai_format_model = is_known_openai_model(&model);

    let port = if is_openai_format_model {
        // Make sure the proxy is up and pick a port.
        let p = Proxy::new(paths).start(port)?;
        let port = match p {
            crate::proxy::ProxyState::Healthy { port, .. } => port,
            _ => {
                eprintln!("error: proxy did not become healthy");
                return Ok(1);
            }
        };
        Some(port)
    } else {
        if let Some(p) = port {
            if !((4141..=4242).contains(&p)) {
                eprintln!("error: invalid port: {p}");
                return Ok(1);
            }
        }
        None
    };

    // Build a synthetic "opencode-go" provider to feed into
    // `settings::turn_on`, since `on` is hardcoded to that provider
    // in the bash contract.
    let provider = if is_openai_format_model {
        provider::built_in_presets()
            .into_iter()
            .find(|p| p.id == "opencode-go")
            .map(|mut p| {
                p.format = ProviderFormat::OpenAI;
                p
            })
            .ok_or_else(|| anyhow::anyhow!("opencode-go preset not found"))?
    } else {
        provider::built_in_presets()
            .into_iter()
            .find(|p| p.id == "opencode-go")
            .ok_or_else(|| anyhow::anyhow!("opencode-go preset not found"))?
    };

    settings::turn_on(
        paths,
        &TurnOnInputs {
            provider: &provider,
            model: &model,
            format: if is_openai_format_model {
                ProviderFormat::OpenAI
            } else {
                ProviderFormat::Anthropic
            },
            port,
            auth_token: &auth,
        },
    )
    .context("write settings.json")?;

    // Marker file so `off` knows to stop the proxy.
    std::fs::create_dir_all(&paths.state_dir).ok();
    std::fs::write(&paths.marker_file, b"").ok();

    println!("OpenCode Go routing enabled");
    println!("  Path:     {}", provider.format.as_str());
    println!("  Model:    {model}");
    if let Some(p) = port {
        println!("  Proxy:    http://127.0.0.1:{p}");
    } else {
        println!("  Base:     {}", provider.base_url);
    }
    println!("  Auth:     OPENCODE_API_KEY env var");
    println!("  Config:   {}", paths.settings_file.display());
    Ok(0)
}

fn cmd_off(paths: &Paths) -> Result<i32> {
    let was_enabled = SettingsState::peek(paths).map(|s| s.enabled).unwrap_or(false);
    settings::turn_off(paths).context("strip settings.json")?;
    if paths.marker_file.exists() {
        if let crate::proxy::ProxyState::Healthy { .. } = Proxy::new(paths).current_state() {
            Proxy::new(paths).stop().context("stop proxy")?;
            println!("proxy stopped");
        }
    }
    let _ = std::fs::remove_file(&paths.marker_file);
    if was_enabled {
        println!("OpenCode Go routing disabled. Claude Code will use the default Anthropic endpoint.");
    } else {
        println!("OpenCode Go routing was already disabled");
    }
    Ok(0)
}

fn cmd_status(paths: &Paths) -> Result<i32> {
    let s = SettingsState::peek(paths).context("read settings")?;
    let proxy = Proxy::new(paths).current_state();
    if s.enabled {
        println!("OpenCode Go is ENABLED");
        println!("  State:    {}", path_kind_str(s.path_kind));
        println!("  Model:    {}", s.model);
        println!("  Base:     {}", s.base_url);
        println!(
            "  Auth:     {}",
            if std::env::var("OPENCODE_API_KEY").is_ok() {
                "OPENCODE_API_KEY env var".to_string()
            } else if s.key_in_settings {
                "ANTHROPIC_AUTH_TOKEN (from settings.json)".to_string()
            } else {
                "not set (verify will fail)".to_string()
            }
        );
        if matches!(s.path_kind, settings::PathKind::OpenAI) {
            match proxy {
                crate::proxy::ProxyState::Healthy { port, pid } => {
                    println!("  Proxy:    running on http://127.0.0.1:{port} (pid {pid})");
                }
                _ => {
                    println!("  Proxy:    EXPECTED BUT NOT RUNNING -- run: claude-go on");
                }
            }
        }
        println!("  Config:   {}", paths.settings_file.display());
    } else {
        println!("OpenCode Go is DISABLED");
        println!("  Endpoint: default Anthropic (api.anthropic.com)");
        println!("  Config:   {}", paths.settings_file.display());
    }
    Ok(0)
}

fn cmd_verify(paths: &Paths) -> Result<i32> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    match runtime.block_on(verify::verify(paths)) {
        Ok(r) => {
            println!("{}", r.outcome.message());
            println!("  Model:    {}", r.model);
            println!("  Base:     {}", r.base_url);
            println!("  HTTP:     {}", r.http_code);
            Ok(if r.outcome.is_ok() { 0 } else { 1 })
        }
        Err(e) => {
            eprintln!("error: {e}");
            Ok(1)
        }
    }
}

fn cmd_models() -> Result<i32> {
    // Iterate the hardcoded 19-model list in stable order (mirrors
    // bash v0.1.1's fixed-order array).
    let preset = provider::built_in_presets()
        .into_iter()
        .find(|p| p.id == "opencode-go")
        .ok_or_else(|| anyhow::anyhow!("opencode-go preset not found"))?;
    let models = match &preset.model_source {
        provider::ModelSource::Live { fallback, .. } => fallback.clone(),
        _ => unreachable!(),
    };
    println!("{:<22} {:<10} DESCRIPTION", "MODEL", "PATH");
    println!("{:<22} {:<10} -----------", "-----", "----");
    for m in &models {
        let path = if m.description.contains("proxy") {
            "openai"
        } else {
            "anthropic"
        };
        println!("{:<22} {:<10} {}", m.id, path, m.description);
    }
    println!();
    println!("default model: minimax-m3 (anthropic path)");
    Ok(0)
}

fn cmd_providers(paths: &Paths) -> Result<i32> {
    let custom = provider::load_custom_providers(&paths.providers_file);
    let list = provider::provider_list(&custom);
    println!("{:<22} {:<10} {:<10} {}", "ID", "FORMAT", "AUTH", "BASE URL");
    println!("{:<22} {:<10} {:<10} {}", "--", "------", "----", "--------");
    for p in &list {
        println!(
            "{:<22} {:<10} {:<10} {}",
            p.id,
            p.format.as_str(),
            p.auth_header,
            if p.base_url.is_empty() {
                "(none)"
            } else {
                p.base_url.as_str()
            }
        );
    }
    Ok(0)
}

fn cmd_provider_add(
    paths: &Paths,
    name: String,
    url: String,
    format: String,
    auth_header: String,
    models: Vec<String>,
) -> Result<i32> {
    if name == CUSTOM_URL_ID {
        eprintln!("error: `{name}` is a reserved id");
        return Ok(1);
    }
    let format = match format.as_str() {
        "anthropic" => ProviderFormat::Anthropic,
        "openai" => ProviderFormat::OpenAI,
        other => {
            eprintln!("error: unknown format `{other}` (use `anthropic` or `openai`)");
            return Ok(1);
        }
    };
    let entry = CustomProviderEntry {
        name: name.clone(),
        base_url: url,
        format,
        auth_header,
        models,
    };
    provider::add_custom_provider(&paths.providers_file, &name, entry)
        .context("write providers.json")?;
    println!("Added provider `{name}`");
    Ok(0)
}

fn cmd_provider_remove(paths: &Paths, name: String) -> Result<i32> {
    provider::remove_custom_provider(&paths.providers_file, &name)
        .context("write providers.json")?;
    println!("Removed provider `{name}`");
    Ok(0)
}

fn cmd_install(paths: &Paths) -> Result<i32> {
    let src = current_exe_path().context("locate current binary")?;
    let target = &paths.install_path;
    if same_file(&src, target).unwrap_or(false) {
        println!("Already installed at {}", target.display());
        return Ok(0);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).context("create ~/.local/bin")?;
    }
    std::fs::copy(&src, target).context("copy binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(target)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(target, perms)?;
    }
    println!("Installed: {}", target.display());
    if let Ok(which) = which("claude-go") {
        println!("Run: {which} --help");
    }
    Ok(0)
}

fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("get current_exe")
}

fn same_file(a: &PathBuf, b: &PathBuf) -> std::io::Result<bool> {
    if !a.exists() || !b.exists() {
        return Ok(false);
    }
    let ma = std::fs::metadata(a)?;
    let mb = std::fs::metadata(b)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

fn which(name: &str) -> Result<String> {
    let path = std::env::var_os("PATH").context("PATH not set")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Ok(cand.display().to_string());
        }
    }
    anyhow::bail!("`{name}` not found on PATH")
}

fn is_known_openai_model(model: &str) -> bool {
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

fn path_kind_str(k: settings::PathKind) -> &'static str {
    match k {
        settings::PathKind::Anthropic => "anthropic",
        settings::PathKind::OpenAI => "openai (via proxy)",
        settings::PathKind::Other => "(none)",
    }
}

fn print_help() {
    println!("claude-go  --  route Claude Code to any Anthropic-compatible model");
    println!();
    println!("USAGE:");
    println!("  claude-go                        Launch the TUI");
    println!("  claude-go on  [--model M] [--port P]   Enable routing");
    println!("  claude-go off                    Disable routing");
    println!("  claude-go status                 Show current state");
    println!("  claude-go verify                 Round-trip test against the live endpoint");
    println!("  claude-go models                 List known models");
    println!("  claude-go providers              List configured providers");
    println!("  claude-go provider add NAME --url URL [--auth-header H] [--format F]");
    println!("  claude-go provider remove NAME   Remove a custom provider");
    println!("  claude-go install                Install to ~/.local/bin");
    println!("  claude-go help                   This help");
    println!();
    println!("ENV:");
    println!("  OPENCODE_API_KEY   Your OpenCode Go key. Required for `on` and `verify`.");
    println!();
    println!("FILES:");
    println!("  Settings:  ~/.claude/settings.json");
    println!("  State dir: ~/.local/share/claude-go/");
    println!("  Providers: ~/.config/claude-go/providers.json");
    println!();
    println!("CAVEATS:");
    println!("  Linux and macOS only in v0.1.0.");
    println!("  The TUI requires a real terminal; for scripts, use the subcommands above.");
    println!("  Sub-tasks (haiku/sonnet/opus) all use the main model.");
}
