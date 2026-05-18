#!/usr/bin/env bash
# Pull a "wild" / unlabeled sample of repos that show an agent commit-signature.
#
# This is the third corpus tier (alongside known-good and slop). Calibration
# discipline:
#   - known-good must stay clean (false-positive guard)
#   - slop must keep firing (false-negative guard)
#   - wild reveals where signals fire on real-world agent-touched code that
#     hasn't been hand-picked — the distribution test
#
# Selection: repos with any commit whose message matches a known agent
# signature, active in the last 90 days, more than one commit, not already in
# the known-good or slop corpora.
#
# Usage:
#   scripts/pull_wild.sh [N]    # N defaults to 5 (smoke test). Raise after.
#
# Output: corpus/wild/<owner>__<repo>/

set -euo pipefail

N="${1:-5}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/corpus/wild"
LOG="$OUT/_pull.log"
mkdir -p "$OUT"

# Lovable / v0 / bolt produce these tells. Editor copilots (cursor/aider) are
# deliberately excluded — they tag *commits in any repo*, including healthy
# ones, which would dilute the signal we're trying to surface.
QUERIES=(
  '"Sync changes" Lovable'
  '"Initial commit by lovable"'
  '"made with bolt"'
  '"Generated with v0"'
)

# Existing corpus repos — never re-pull these.
seen_owners_and_names() {
  find "$ROOT/corpus/known-good" "$ROOT/corpus/slop" \
       -maxdepth 4 -name config -path '*/.git/config' 2>/dev/null \
    | xargs grep -h "url = " 2>/dev/null \
    | sed -E 's|.*github\.com[/:]([^/]+)/([^/. ]+).*|\1/\2|' \
    | tr '[:upper:]' '[:lower:]' \
    | sort -u
}

EXISTING="$(seen_owners_and_names)"

is_already_corpus() {
  local repo="$(echo "$1" | tr '[:upper:]' '[:lower:]')"
  echo "$EXISTING" | grep -qx "$repo"
}

# Collected repo full-names, deduplicated.
candidates_file="$(mktemp)"
trap 'rm -f "$candidates_file"' EXIT

echo "[pull_wild] querying commit signatures..." | tee -a "$LOG" >&2
for q in "${QUERIES[@]}"; do
  echo "  $q" | tee -a "$LOG" >&2
  # gh search commits requires a cloak header; --json takes care of it.
  gh search commits "$q" --limit 100 --json repository \
      --jq '.[].repository.fullName' 2>>"$LOG" \
    || echo "  (query failed; continuing)" | tee -a "$LOG" >&2
done >> "$candidates_file"

# Dedup and shuffle so the first N isn't always the same repos.
sort -uR "$candidates_file" > "${candidates_file}.uniq"
total="$(wc -l < "${candidates_file}.uniq" | tr -d ' ')"
echo "[pull_wild] $total unique candidates; selecting up to $N" | tee -a "$LOG"

cutoff_iso="$(date -u -v-90d '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
            || date -u -d '90 days ago' '+%Y-%m-%dT%H:%M:%SZ')"

picked=0
while IFS= read -r repo; do
  [ "$picked" -ge "$N" ] && break
  [ -z "$repo" ] && continue
  if is_already_corpus "$repo"; then
    echo "  skip (already in corpus): $repo" | tee -a "$LOG"
    continue
  fi
  # Metadata: must have >1 commit and pushed within 90 days. A single
  # `gh api repos/$repo` call gets us both pushedAt and default branch.
  meta="$(gh api "repos/$repo" 2>>"$LOG" || true)"
  if [ -z "$meta" ]; then
    echo "  skip (api fail): $repo" | tee -a "$LOG"
    continue
  fi
  pushed_at="$(echo "$meta" | jq -r '.pushed_at // empty')"
  archived="$(echo "$meta"  | jq -r '.archived // false')"
  size_kb="$(echo "$meta"   | jq -r '.size // 0')"
  if [ "$archived" = "true" ] || [ "$pushed_at" \< "$cutoff_iso" ]; then
    echo "  skip (inactive/archived): $repo" | tee -a "$LOG"
    continue
  fi
  # >500 MB repos are mostly assets and slow to clone; skip the long tail.
  if [ "${size_kb:-0}" -gt 500000 ]; then
    echo "  skip (too big: ${size_kb} KB): $repo" | tee -a "$LOG"
    continue
  fi

  owner="${repo%%/*}"
  name="${repo##*/}"
  dest="$OUT/${owner}__${name}"
  if [ -d "$dest/.git" ]; then
    echo "  skip (already cloned): $repo" | tee -a "$LOG"
    continue
  fi

  echo "[pull_wild] cloning $repo ..." | tee -a "$LOG"
  if git clone --quiet "https://github.com/$repo.git" "$dest" 2>>"$LOG"; then
    commits="$(git -C "$dest" rev-list --count HEAD 2>/dev/null || echo 0)"
    if [ "${commits:-0}" -le 1 ]; then
      echo "  drop (single-commit dump): $repo" | tee -a "$LOG"
      rm -rf "$dest"
      continue
    fi
    picked=$((picked + 1))
    echo "  ok ($commits commits): $repo" | tee -a "$LOG"
  else
    echo "  clone failed: $repo" | tee -a "$LOG"
    rm -rf "$dest"
  fi
done < "${candidates_file}.uniq"

echo "[pull_wild] picked $picked repo(s) into $OUT" | tee -a "$LOG"
