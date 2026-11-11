#!/usr/bin/env bash
# The one entry point that builds testroms/ (AGENTS.md section 1). Local runs, ./scripts/verify.sh
# and the CI `testroms` job all call this script, so they use the same toolchain.
set -euo pipefail

# BlocksDS slim-v1.24.0 — the multi-arch index digest (amd64 + arm64), not a per-platform one.
# Bumping it is an `UPT:` PR; every golden hash it changes needs its reason (AGENTS.md section 7).
BLOCKSDS_IMAGE="skylyrac/blocksds@sha256:779f6193d1030c62e60d1f0f39c3e32cf4045c0bd224a45c0ceadf9d9ab01a51"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v docker >/dev/null 2>&1 || { echo "docker not found — install Docker to build the test ROMs" >&2; exit 1; }

docker run --rm -v "$ROOT:/work" -w /work --user "$(id -u):$(id -g)" \
  "$BLOCKSDS_IMAGE" make -C testroms "$@"
