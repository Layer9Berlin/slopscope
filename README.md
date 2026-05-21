# slopscope

A tool that detects "slop" in AI/vibe-coded codebases — and, run live during a coding
session, steers the agent off the slop path before the rot sets in.

It is not another SonarQube. The novel part is **process archaeology**: reading git
history and the filesystem to diagnose *how a repo got into its state* — fix-loops,
proliferating status docs, committed backups, accidental files — signals that
code-only analyzers structurally cannot see.

## Two modes, one engine

- **`audit`** — full-repo scan → structured findings + a human-readable report.
- **`guard`** — incremental, diff-scoped, fast; wired as an editor/agent hook so the
  agent self-corrects mid-session.

## Install

macOS / Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Layer9Berlin/slopscope/releases/latest/download/slopscope-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/Layer9Berlin/slopscope/releases/latest/download/slopscope-installer.ps1 | iex
```

The installer detects your platform and downloads the matching prebuilt
binary. Per-platform archives are also attached to every
[release](https://github.com/Layer9Berlin/slopscope/releases).

From source, with a Rust toolchain: `cargo install --path .`.

## Integrations

slopscope runs as an MCP server (`slopscope mcp`, stdio transport), so any
MCP-aware agent can call the audit as a tool.

- **Claude Code** — install the bundled plugin:

  ```
  /plugin marketplace add Layer9Berlin/slopscope
  /plugin install slopscope@layer9
  ```

  The plugin registers the MCP server and adds a `/slopscope:audit` command.
  See `integrations/claude-plugin/`.

- **Codex** — add the MCP server to `~/.codex/config.toml`; the ready-to-merge
  snippet is in `integrations/codex/config.toml`.

- **Other MCP clients** — point the client at `slopscope mcp`.

## Status

Early. Spec is locked; building the Phase-1 deterministic core (VCS hygiene +
steering-failure signature) plus the measurement harness.

## License

MIT
