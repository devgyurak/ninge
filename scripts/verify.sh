#!/usr/bin/env bash
# The one gate (AGENTS.md §1). CI (.github/workflows/ci.yml) runs this same script.
#
# Steps run in order and stop at the first failure: a fix in an earlier step changes a later
# step's result. Until Phase 0 adds Cargo.toml, the cargo steps print SKIPPED — they did not
# pass, they did not run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PYTHON="${PYTHON:-python3}"

step() { printf '\n▶ %s\n' "$1"; }
skip() { printf '  SKIPPED: %s\n' "$1"; }

step "agent config"
"$PYTHON" scripts/check_agent_config.py
step "hook selftest"
"$PYTHON" scripts/agent_hooks/selftest.py
step "clean-room scan (tracked files)"
"$PYTHON" scripts/agent_hooks/guard_cleanroom.py --scan-tracked
step "architecture (core purity, crate graph, names)"
"$PYTHON" scripts/agent_hooks/check_architecture.py --all
step "test conventions (GWT names, no control flow, asserts)"
"$PYTHON" scripts/agent_hooks/check_tests.py --all
step "language (English only, translations have an original)"
"$PYTHON" scripts/agent_hooks/check_language.py --all

if [ ! -f Cargo.toml ]; then
  step "cargo fmt · clippy · deny · tests + coverage"
  skip "no Cargo workspace yet (Phase 0 adds it)"
  printf '\nverify: Python gates passed; Rust gates skipped.\n'
  exit 0
fi

command -v cargo >/dev/null 2>&1 || { echo "cargo not found — install the Rust toolchain" >&2; exit 1; }

step "pinned toolchain"
# AGENTS.md §4: an exact version, never a floating channel.
if ! grep -Eq '^channel[[:space:]]*=[[:space:]]*"1\.[0-9]+\.[0-9]+"' rust-toolchain.toml 2>/dev/null; then
  echo 'rust-toolchain.toml must pin an exact version: channel = "1.NN.0"' >&2
  exit 1
fi
echo "  $(grep -E '^channel' rust-toolchain.toml)"

step "cargo fmt"
cargo fmt --all -- --check
step "cargo clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings

need() {
  "$@" >/dev/null 2>&1 || { printf '%s is required: %s\n' "$1 $2" "$INSTALL" >&2; exit 1; }
}

step "dependency licenses and sources (cargo-deny)"
INSTALL="cargo install --locked cargo-deny"
need cargo deny --version
cargo deny --locked check licenses bans sources

# Test ROMs (AGENTS.md §3.1, §7). Built ROMs are never committed, so the golden tests need
# them built from testroms/ sources. Decided in #65.
#   1. NINGE_PREBUILT_TESTROMS=1 -> verify the prebuilt artifact's checksums (the macOS CI
#      job, which cannot run Docker). Only ci.yml sets it, so a stale local build/ directory
#      can never pass.
#   2. a running Docker daemon   -> rebuild from source with the pinned BlocksDS image (-B
#      ignores timestamps, so nothing stale is trusted). If NINGE_EXPECTED_TESTROM_SUMS names
#      the checksums of the CI artifact, the rebuild must match them byte for byte.
#   3. otherwise                 -> fail. A missing toolchain is never a pass.
step "test ROMs (BlocksDS)"
if [ "${NINGE_PREBUILT_TESTROMS:-}" = "1" ]; then
  [ -f testroms/build/SHA256SUMS ] || { echo "testroms/build/SHA256SUMS is missing — the testroms artifact was not downloaded" >&2; exit 1; }
  (cd testroms/build && shasum -a 256 -c SHA256SUMS)
  echo "  verified prebuilt test ROMs (not built on this runner)"
elif command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  ./scripts/build_testroms.sh -B
  if [ -n "${NINGE_EXPECTED_TESTROM_SUMS:-}" ]; then
    diff "$NINGE_EXPECTED_TESTROM_SUMS" testroms/build/SHA256SUMS \
      || { echo "rebuilt test ROMs differ from the testroms artifact — the build is not reproducible" >&2; exit 1; }
    echo "  rebuilt test ROMs match the testroms artifact"
  fi
else
  echo "A running Docker is required to build the test ROMs — install and start Docker (https://docs.docker.com/get-docker/) and retry." >&2
  exit 1
fi

# Line-coverage floor (AGENTS.md §5.9). scripts/check_agent_config.py fails if this number
# and the one in AGENTS.md differ — lowering it is a decision, not an edit.
COVERAGE_FLOOR=90
step "tests + coverage (floor ${COVERAGE_FLOOR}%)"
INSTALL="cargo install --locked cargo-llvm-cov && rustup component add llvm-tools-preview"
need cargo llvm-cov --version
if cargo nextest --version >/dev/null 2>&1; then
  cargo llvm-cov nextest --workspace --locked --fail-under-lines "$COVERAGE_FLOOR"
else
  cargo llvm-cov --workspace --locked --fail-under-lines "$COVERAGE_FLOOR"
fi
step "doc tests"
cargo test --workspace --locked --doc

printf '\nverify: all gates passed.\n'
