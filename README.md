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

From a clone, with a Rust toolchain:

```sh
cargo install --path .
```

Prebuilt binaries for macOS, Linux, and Windows are published on the GitHub
Releases page (built by the `release` workflow). Download the archive for your
platform and put the `slopscope` binary on your `PATH`.

## Integrations

slopscope runs as an MCP server (`slopscope mcp`, stdio transport), so any
MCP-aware agent can call the audit as a tool.

- **Claude Code** — install the bundled plugin. With the repo as a local
  marketplace:

  ```
  /plugin marketplace add /path/to/slopscope
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

TBD (leaning permissive / MIT — this is an open-source reputation project).
