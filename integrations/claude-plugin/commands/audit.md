---
description: Audit a repository for AI/vibe-coding "slop"
---

Audit a repository for "slop" — the structural signature of AI / vibe-coded
code — and report the findings.

Target: if `$ARGUMENTS` is non-empty, treat it as the path to audit; otherwise
audit the current working directory.

Steps:
1. Call the `audit` tool from the slopscope MCP server with the absolute path
   to the target. If the MCP server is unavailable, run `slopscope audit
   <path>` in the shell instead.
2. Group the findings by severity — critical first, then warn, then info.
3. For each critical and warn finding, give: the check id, the concrete
   evidence slopscope reported (file paths, commit hashes, counts), and one
   specific fix.
4. End with slopscope's one-line severity summary.

slopscope output is deterministic ground truth — report the counts and
evidence as-is. Do not soften, reinterpret, or vibe past them.
