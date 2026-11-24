# Commands

The full command list for working on ninge. The rules that go with it (the gate must pass
before a task is done, no routing around a failure) stay in `AGENTS.md` §1.

```bash
./scripts/verify.sh                                        # the whole gate, in order — CI runs the same script
./scripts/build_testroms.sh                                # build testroms/ into git-ignored testroms/build/ (needs Docker)
./scripts/fetch_cpu_suite.sh                               # fetch the ARM7TDMI JSON suite into git-ignored tests/data/ (checksummed)
cargo fmt --all                                            # format
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo deny --locked check licenses bans sources            # dependency licenses (deny.toml)
cargo llvm-cov nextest --workspace --locked --lib --bins   # unit tests + coverage
python3 scripts/check_agent_config.py                      # agent-config integrity
python3 scripts/agent_hooks/selftest.py                    # hook self-test
python3 scripts/agent_hooks/guard_cleanroom.py --scan-tracked   # no dumps / GPL text tracked
python3 scripts/agent_hooks/check_architecture.py --all    # core purity · crate graph · names · SPDX
python3 scripts/agent_hooks/check_tests.py --all           # GWT names · no control flow · asserts
python3 scripts/agent_hooks/check_language.py --all        # English-only text · translations have an original
```

Recording and replaying input: the log format is in [input-log.md](input-log.md).

Until Phase 0 landed the Cargo workspace, `verify.sh` ran the Python gates and said plainly
that it skipped the cargo gates; a skipped step is never reported as passed.

A documentation-only change runs only the Python gates; see
[ci-and-release.md](ci-and-release.md#documentation-only-changes).
