# slopscope — project context for Claude

`slopscope` detects "slop" in AI/vibe-coded codebases and (in `guard` mode) steers
coding agents off the slop path live. See `README.md` for the pitch.

## Where the thinking lives

The full Phase-1 spec — categories, metrics, the `audit`/`guard` modes, scoring,
phasing, test corpus — is in auto-memory: `project_slop_detector_spec.md`
(plus `project_slop_detector.md` for background and strategy). Read those before
making architecture decisions; they hold context that predates this repo.

## Key decisions already locked

- **Two modes, one metric engine:** `audit` (full-repo, diagnostic) and `guard`
  (incremental, hook-driven, preventive).
- **Deterministic-first:** tools → verify in code → LLM only at the edges.
- **Wrap, don't build** for commodity analysis (code smells, deps). Build the novel
  buckets: A (VCS hygiene) and B (steering-failure signature).
- **Stack:** leaning Rust; Go is the fallback.
- Output must be deterministic ground truth an agent can't vibe past — counts, file
  lists, commit sequences, thresholds. Not opinions.

## Scratch

Local audit corpus and throwaway analysis go in `/scratch` or `/corpus` (gitignored).
Earlier manual audits were done in `/tmp/slop-audit/`.
