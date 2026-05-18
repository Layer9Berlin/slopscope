#!/usr/bin/env bash
# Aggregate slopscope findings across a corpus dir.
#
# Usage:
#   scripts/aggregate.sh <corpus-dir> [out-prefix]
#
# Reads every `<corpus-dir>/*/` as a git repo, runs slopscope --json on each,
# and prints:
#   - per-signal: how many repos fired, severity histogram
#   - distribution of total finding count per repo
#   - repos firing nothing (potential drop-from-tier candidates)
#   - top-firing repos (potential promote-to-slop candidates)
#
# Audits are cached in `<corpus-dir>/_audits/<repo>.json` so re-runs are fast.

set -euo pipefail

CORPUS="${1:?usage: aggregate.sh <corpus-dir>}"
[ -d "$CORPUS" ] || { echo "no such dir: $CORPUS" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/slopscope"
[ -x "$BIN" ] || { echo "build slopscope --release first" >&2; exit 1; }

AUDITS="$CORPUS/_audits"
mkdir -p "$AUDITS"

# Collect audits, caching by repo dir name.
repo_count=0
for d in "$CORPUS"/*/; do
  [ -d "$d/.git" ] || continue
  name="${d%/}"; name="${name##*/}"
  out="$AUDITS/${name}.json"
  if [ ! -s "$out" ]; then
    if ! "$BIN" audit "$d" --json > "$out" 2>/dev/null; then
      # Non-zero exit codes are normal (1 = warn, 2 = crit). Only drop on 3 = error.
      rc=$?
      if [ "$rc" -eq 3 ]; then
        rm -f "$out"
        continue
      fi
    fi
  fi
  repo_count=$((repo_count + 1))
done
echo "[aggregate] $repo_count repo(s) audited"
echo

# Per-signal counts via jq across all audit files.
echo "===== per-signal firing rates ====="
printf "%-30s %5s %5s %5s %5s\n" "check" "fires" "crit" "warn" "info"
jq -r --slurpfile a <(jq -s . "$AUDITS"/*.json) '
  $a[0]
  | map(.findings[])
  | group_by(.check)
  | map({
      check: .[0].check,
      fires: length,
      crit: ([.[] | select(.severity=="critical")] | length),
      warn: ([.[] | select(.severity=="warn")]     | length),
      info: ([.[] | select(.severity=="info")]     | length),
    })
  | sort_by(-.fires)
  | .[]
  | "\(.check)\t\(.fires)\t\(.crit)\t\(.warn)\t\(.info)"
' <<< 'null' \
| while IFS=$'\t' read -r check fires crit warn info; do
    printf "%-30s %5s %5s %5s %5s\n" "$check" "$fires" "$crit" "$warn" "$info"
  done
echo

# Per-repo total findings — to find quiet repos (no signal) and loud outliers.
echo "===== per-repo finding count ====="
for f in "$AUDITS"/*.json; do
  name=$(basename "$f" .json)
  n=$(jq '.findings | length' < "$f")
  crit=$(jq '[.findings[] | select(.severity=="critical")] | length' < "$f")
  warn=$(jq '[.findings[] | select(.severity=="warn")]     | length' < "$f")
  printf "%s\t%s\t%s\t%s\n" "$n" "$crit" "$warn" "$name"
done | sort -nr | awk -F'\t' 'BEGIN{print "n\tcrit\twarn\trepo"} {print}' \
     | column -t -s $'\t'
echo

# Quiet repos: zero findings = either truly clean OR signals don't apply.
echo "===== repos firing nothing ====="
for f in "$AUDITS"/*.json; do
  n=$(jq '.findings | length' < "$f")
  if [ "$n" = "0" ]; then
    echo "  $(basename "$f" .json)"
  fi
done
