#!/usr/bin/env bash
# Apply .github/rulesets/*.json to the GitHub repository (a ruleset with the same name is
# updated in place). Needs `gh` authenticated as a repository admin.
#
#   .github/rulesets/apply.sh [owner/repo]
#
# Two rulesets cover `main` and `develop` (AGENTS.md §7):
#   branches — no deletion, no force push, required CI checks. Nobody bypasses it: even the
#              admin merges only on green CI.
#   reviews  — changes land through a PR with one approval. The repository admin role
#              (actor_id 5; on this personal repository, only the owner) may bypass it, and
#              only when merging a PR (`bypass_mode: pull_request`) — direct pushes stay
#              blocked.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

repo="${1:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"

for f in *.json; do
  name=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["name"])' "$f")
  id=$(gh api "repos/$repo/rulesets" --jq ".[] | select(.name == \"$name\") | .id")
  if [ -n "$id" ]; then
    gh api -X PUT "repos/$repo/rulesets/$id" --input "$f" >/dev/null
    printf '  updated %-8s (#%s)\n' "$name" "$id"
  else
    id=$(gh api -X POST "repos/$repo/rulesets" --input "$f" --jq .id)
    printf '  created %-8s (#%s)\n' "$name" "$id"
  fi
done
