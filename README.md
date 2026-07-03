# claude-go

Route Claude Code to any Anthropic-compatible model with a beautiful TUI.

```text
claude-go  /  provider                                                  1  .  2  .  3
──────────────────────────────────────────────────────────────────────
 >>    OpenCode Go             [anthropic]  https://opencode.ai/zen/go
       Anthropic direct        [anthropic]  https://api.anthropic.com
       OpenRouter              [anthropic]  https://openrouter.ai/api/v1
    !  Cloudflare AI Gateway   [anthropic]  https://gateway.ai.cloudflare.com/v1
    !  Google Vertex (Claude)  [anthropic]  https://aiplatform.googleapis.com
    !  AWS Bedrock (Claude)    [anthropic]  https://bedrock-runtime.us-east-1.amazonaws.com
       Custom URL...           [anthropic]  ...
──────────────────────────────────────────────────────────────────────
j/k or arrows  move     Enter  select     a  add custom     d  remove custom
```

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/Abdk4Moura/claude-go-rs/main/install.sh | bash
```

This downloads the right binary for your OS/arch from the latest GitHub
release and installs it to `~/.local/bin/claude-go`. Make sure
`~/.local/bin` is on your `PATH`.

Want a specific version?

```sh
curl -fsSL https://raw.githubusercontent.com/Abdk4Moura/claude-go-rs/main/install.sh | bash -s -- v0.1.0
```

## Quick start

```sh
# 1. Get an API key (https://opencode.ai/auth), then:
export OPENCODE_API_KEY=sk-...

# 2. Launch the TUI
claude-go

# 3. Pick a provider, pick a model, and you're routed.
```

Or stay on the command line:

```sh
claude-go on --model minimax-m3
claude-go status
claude-go verify
claude-go off
```

## What it does

`claude-go` writes the right `ANTHROPIC_*` env vars into
`~/.claude/settings.json` so Claude Code routes to a different model. It
owns a small, well-defined slice of that file (10 env keys + a marker)
and touches nothing else -- your other env vars, permissions, theme,
plugins, MCP servers, and hooks stay put.

Two endpoint shapes are supported out of the box:

- **Anthropic-format** (e.g. OpenCode Go's `/v1/messages`, direct
  Anthropic, OpenRouter): direct, no proxy.
- **OpenAI Chat Completions format** (e.g. GLM, Kimi, DeepSeek via
  OpenCode Go): routed through the local `opencode-api` translation
  proxy that `claude-go on` starts and `claude-go off` stops.

The proxy lifecycle is fully managed: `claude-go` picks a free port in
`4141..4242`, spawns `opencode-api` under `setsid(2)` so it survives
shell teardown, polls `/health` to confirm it's up, and on `off` sends
`SIGTERM` to the process group (so all the Node workers die too) with
a `SIGKILL` fallback after 2s.

## Architecture

```text
Claude Code
    |
    |  Anthropic Messages API
    v
+--------------------+
| claude-go          |
| (settings.json)    |
+--------------------+
    |
    |  Anthropic-format  -> direct to provider
    |  OpenAI-format     -> opencode-api proxy (localhost:????)
    v
+--------------------+    +------------------+
| Provider           |    | opencode-api     |
| (OpenCode Go,      |    | (Node)           |
|  OpenRouter, ...)  |    | translation proxy|
+--------------------+    +------------------+
```

`~/.claude/settings.json` is the only thing Claude Code reads. `claude-go`
writes a 10-key env block with an `__claude_go_owned: "1"` marker so
`off` only strips its own keys and never destroys a user's own
`ANTHROPIC_*` setup.

## TUI

Three screens, navigated with Tab / arrow keys + Enter. Quit with `q`
or `Ctrl-C`.

| Screen | Purpose |
|--------|---------|
| 1 / provider | Pick a built-in preset (OpenCode Go, Anthropic direct, OpenRouter, ...) or add a custom URL. |
| 2 / model | Pick a model. For OpenCode Go the list is live-fetched from `/v1/models` (5 min cache) and falls back to the hardcoded 19 models. For other providers, type any model id. |
| 3 / status | Big ON / OFF indicator + live state (settings.json path, proxy state, last verify result). `o` toggles, `v` runs verify, `r` refreshes. |

The TUI is a real TUI. It uses crossterm + ratatui, supports any
terminal width, and degrades to a clean CLI for scripting.

## CLI

```
claude-go                          # launch the TUI
claude-go on [--model M] [--port P] # enable
claude-go off                      # disable
claude-go status                   # show current state
claude-go verify                   # round-trip test
claude-go models                   # list 19 hardcoded models
claude-go providers                # list configured providers
claude-go provider add NAME --url URL [--auth-header H]
claude-go provider remove NAME
claude-go install                  # install to ~/.local/bin
claude-go help                     # help
claude-go version                  # version
```

## Custom providers

```sh
claude-go provider add my-corp --url https://llm.internal.corp
claude-go provider remove my-corp
```

Persists to `~/.config/claude-go/providers.json`. Custom providers
can be removed from the TUI with `d`.

## Files

| Path | What |
|------|------|
| `~/.claude/settings.json` | Claude Code's settings (claude-go owns a 10-key env block) |
| `~/.local/share/claude-go/` | State dir (proxy.pid, proxy.port, proxy.log, marker) |
| `~/.config/claude-go/providers.json` | Custom provider registry |
| `~/.local/bin/claude-go` | Default install path |

## Requirements

- Linux (x86_64 or aarch64) or macOS (x86_64 or Apple Silicon)
- Node 18+ (for the `opencode-api` translation proxy when you pick an
  OpenAI-format model)
- `OPENCODE_API_KEY` in your environment for OpenCode Go

Windows is not supported in v0.1.0.

## Caveats

- The TUI is a real TUI. If you're piping into a script, use the
  subcommands instead -- they print plain text.
- Sub-tasks (haiku/sonnet/opus routing) all use the main model. There's
  no per-subtask dispatch in this tool.
- Cloudflare, Vertex, and Bedrock presets are listed in the TUI but
  show "not yet implemented". OpenCode Go, Anthropic direct, and
  OpenRouter are fully working in v0.1.0.
- The TUI no-arg default is a breaking change from the bash v0.1.1
  tool, which printed help. Users with muscle memory for `claude-go`
  printing help will get a TUI.

## About the rename

This is the Rust port of `claude-go`. It currently lives at
[`Abdk4Moura/claude-go-rs`](https://github.com/Abdk4Moura/claude-go-rs)
because GitHub wouldn't let us reuse the `claude-go` name while the
bash version was still published. The plan is to:

1. Cut v0.1.0 from this repo (the Rust port).
2. Use the Rust port for a few weeks.
3. Once the bash version is fully deprecated, rename the repo to
   `claude-go` and cut v0.2.0.

Until then, `curl ... | bash` from this repo gets you the Rust tool.

## Development

```sh
cargo build --release
cargo test
cargo run -- status
cargo run -- models
```

The dev binary is at `target/release/claude-go`. Run it directly while
iterating; the TUI works in any modern terminal.

## License

MIT. See [LICENSE](./LICENSE).
