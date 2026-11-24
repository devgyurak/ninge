# CI, releases, and the agent gates

Reference detail behind `AGENTS.md` §7. The rules stay in `AGENTS.md`; the steps live here.

## Releases

Decided: SemVer `0.x`; a `vX.Y.Z` tag on `main` is the release.

- The release PR (`develop` → `main`) bumps `[workspace.package] version` **and** the
  `ninge-core` entry's `version` in `[workspace.dependencies]`, sets `[workspace.package]
  publish` to `true`, and refreshes `Cargo.lock` (`cargo update -w`).
- The first release, `0.1.0`, follows the Phase 3 DoD.
- The tag runs `.github/workflows/release.yml`: one `cargo publish` of **ninge-core and
  ninge** (`cargo install ninge`), after a reviewer approves the `crates-io` environment that
  holds the token.
- **ninge-runner and ninge-tracediff are not published** (decided 2026-11-11): their tests
  need the repository's `testroms/`, and publishing would put their command line and report
  format under SemVer while both still change.
- macOS arm64 and Linux x86_64 binaries on GitHub Releases and a Homebrew tap come later.

## Branches

Working PRs target `develop`; only release PRs (`develop` → `main`) target `main`. Both
branches are protected by the rulesets in `.github/rulesets/` (applied with `apply.sh`): no
deletion or force push, green CI required with **no bypass**, and a PR with one approval —
which the admin (`devgyurak`) may bypass when merging a PR. Direct pushes are blocked for
everyone. The repository is `github.com/devgyurak/ninge`.

## CI

`.github/workflows/ci.yml` runs `./scripts/verify.sh` on Linux and macOS: for a PR, a push to
`main`, and a `develop` merge that changes dependencies (`cache.yml`). Tests run in shards
(`NINGE_TEST_SHARD`), the coverage floor checked over all of them.

### Documentation-only changes

A documentation-only change (`scripts/docs_only.py`: `docs/`, the agent settings, Markdown
outside `crates/`) runs only the Python gates; `verify.sh` says that it skipped the cargo
steps. Set `NINGE_DOCS_ONLY_BASE=<commit>` to get the same locally.

## BlocksDS and the test ROMs

BlocksDS is required (decided). Built test ROMs are never committed (`AGENTS.md` §3.1), so the
golden tests, and so the 90% coverage floor, need the SDK.

- It comes from the official BlocksDS Docker image **pinned by digest**, the same locally and
  in CI. Code reads the digest only from `scripts/build_testroms.sh`; `docs/STATUS.md` records
  it and `check_agent_config.py` keeps the two equal.
- Bumping the digest is an `UPT:` PR, and every golden hash it changes needs its reason.
- macOS runners cannot run Docker, so the `testroms` job builds the ROMs once on ubuntu.
  Ubuntu rebuilds from source and must match its `SHA256SUMS`; macOS checks the artifact
  against it (`NINGE_PREBUILT_TESTROMS=1`, set in `ci.yml` only).
- Without both Docker and that variable, `verify.sh` fails, never skips.

## Agent config

`.github/workflows/agents.yml` runs `scripts/check_agent_config.py` and the hook self-test, so
a broken pointer or hook fails in the PR. `check_agent_config.py` imports the hooks' lists to
prove `AGENTS.md` §2 and §3.3 state the same ones, and fails if the coverage floor in §5.9 and
the one in `scripts/verify.sh` differ.

## Permission policy

One policy across harnesses, wired in `.claude/settings.json` and `.opencode/opencode.json`;
Pi and Codex follow their own settings, and the shared guard still runs.

- **Allow** read-only verification (`cargo fmt|clippy|check|test|nextest|llvm-cov|tree|doc`,
  `python3 scripts/…`, `./scripts/verify.sh`, `git status|diff|log|show`) and
  `git add|commit|push` (an agent still commits or pushes only when the user asks).
- **Ask** for dependency and outward-facing commands (`cargo add`, `cargo install`,
  `cargo update`, `gh issue create`, `gh pr create`).
- **Deny** destructive ones (`rm -rf`, force-push).
