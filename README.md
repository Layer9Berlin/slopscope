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

## Status

Early. Spec is locked; building the Phase-1 deterministic core (VCS hygiene +
steering-failure signature) plus the measurement harness.

## License

TBD (leaning permissive / MIT — this is an open-source reputation project).
