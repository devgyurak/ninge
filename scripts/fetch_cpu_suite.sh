#!/usr/bin/env bash
# Fetches the ARM7TDMI JSON test suite into tests/data/ (git-ignored; AGENTS.md section 5.1).
# ./scripts/verify.sh calls it before the tests: a file whose checksum already matches is
# kept, anything else is downloaded, and a checksum that still differs fails the run.
set -euo pipefail

# SingleStepTests/ARM7TDMI (MIT), pinned to one commit. This is the only place the commit
# lives in code; docs/STATUS.md records it and check_agent_config.py keeps the two equal.
SUITE_COMMIT="e3097d88d428752736b949d94c094d5da50f0db6"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUMS="$ROOT/tests/cpu_suite/SHA256SUMS"
DEST="$ROOT/tests/data/arm7tdmi/v1"
URL="https://raw.githubusercontent.com/SingleStepTests/ARM7TDMI/$SUITE_COMMIT/v1"

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

command -v curl >/dev/null 2>&1 || { echo "curl not found — install curl to fetch the CPU test suite" >&2; exit 1; }
mkdir -p "$DEST"

fetched=0
kept=0
while read -r want name; do
  file="$DEST/$name"
  if [ -f "$file" ] && [ "$(sha256 "$file")" = "$want" ]; then
    kept=$((kept + 1))
    continue
  fi
  echo "  fetching $name"
  curl -fsSL --retry 3 --connect-timeout 30 --max-time 600 -o "$file.part" "$URL/$name"
  mv "$file.part" "$file"
  if [ "$(sha256 "$file")" != "$want" ]; then
    echo "checksum mismatch for $name — expected $want" >&2
    rm -f "$file"
    exit 1
  fi
  fetched=$((fetched + 1))
done < "$SUMS"

echo "  CPU suite at ${SUITE_COMMIT:0:7}: $kept already present, $fetched fetched ($DEST)"
