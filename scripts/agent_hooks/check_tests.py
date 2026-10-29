#!/usr/bin/env python3
"""Post-hook (and CI gate) for the Rust test conventions (AGENTS.md §5).

For every `#[test]`-like function in an edited `.rs` file:

  * the name is `given_<state>_when_<action>_then_<observable>`, and the part after the
    last `_then_` is an observable result, not `works` / `ok` / `passes`;
  * the body has no control flow — `if` / `else` / `for` / `while` / `loop` / `match`
    (so no `let … else` either) and no `for_each`. Many cases go through a harness function
    that returns the list of failures, and the test asserts that list is empty;
  * the body asserts something (`assert*!`, `prop_assert*!`, an insta `assert_*_snapshot!`,
    or a helper named `assert_*`), or the fn is `#[should_panic(expected = …)]`.

Modes:
  * default        — judge the `.rs` paths in the hook payload;
  * `--event stop` — Cursor's `stop` payload has no paths; read changed files from git;
  * `--all`        — scan every `.rs` file in the repository and exit non-zero on a finding
                     (what ./scripts/verify.sh and CI run).

The decision logic lives here only; the CI gate is this same script with `--all`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _hookio import (
    changed_paths,
    emit_allow,
    emit_block,
    emit_note,
    extract_paths,
    has_flag,
    hook_root,
    parse_args,
    parse_option,
    read_payload,
    repo_files,
)
from _rustscan import strip, test_functions

GWT_RE = re.compile(r"^given_[a-z0-9_]+_when_[a-z0-9_]+_then_[a-z0-9_]+$")

# A `then_` that names no observable result says nothing a reader can check.
VAGUE_THEN = frozenset(
    {"works", "work", "ok", "okay", "success", "succeeds", "pass", "passes", "good", "correct", "fine", "valid", "done"}
)

CONTROL_FLOW_RE = re.compile(r"\b(if|else|for|while|loop|match|for_each|try_for_each)\b")
ASSERT_RE = re.compile(r"\b(?:prop_)?assert\w*!|\bassert_\w+\s*\(|\binsta::assert_\w+!")

RULE = (
    "Test conventions (AGENTS.md §5): name tests given_<state>_when_<action>_then_<observable>; "
    "keep control flow out of test bodies (put the loop in a harness fn that returns the "
    "failures, then assert the list is empty); every test asserts something."
)


def check_source(source: str, label: str) -> list[str]:
    """Convention violations in one Rust source text. Pure, so the selftest can feed it text."""
    violations: list[str] = []
    for test in test_functions(strip(source)):
        where = f"  {label}:{test.line} `{test.name}`"
        if not GWT_RE.match(test.name):
            violations.append(f"{where} — not given_<state>_when_<action>_then_<observable>.")
        else:
            then_part = test.name.rsplit("_then_", 1)[1]
            if then_part in VAGUE_THEN:
                violations.append(f"{where} — `then_{then_part}` is not an observable result.")
        flow = sorted({m.group(1) for m in CONTROL_FLOW_RE.finditer(test.body)})
        if flow:
            violations.append(f"{where} — control flow in the test body: {', '.join(flow)}.")
        if not ASSERT_RE.search(test.body) and "should_panic" not in test.attrs:
            violations.append(f"{where} — asserts nothing.")
    return violations


def check_file(path: Path) -> list[str]:
    if path.suffix != ".rs" or not path.is_file():
        return []
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    root = hook_root()
    try:
        label = path.resolve().relative_to(root).as_posix()
    except ValueError:
        return []  # outside the repository
    return check_source(source, label)


def main() -> None:
    tool = parse_args()
    root = hook_root()

    if has_flag("--all"):
        violations = [v for path in repo_files(root, ".rs") for v in check_file(path)]
        if violations:
            print("test conventions FAILED:\n" + "\n".join(violations) + f"\n\n{RULE}", file=sys.stderr)
            raise SystemExit(1)
        print("test conventions: all checks passed")
        raise SystemExit(0)

    payload = read_payload()
    event = parse_option("--event")
    paths = changed_paths(root) if event == "stop" else extract_paths(payload)
    violations = [v for raw in paths for v in check_file(Path(raw).expanduser())]
    if not violations:
        emit_allow(tool)

    loop_count = payload.get("loop_count")
    if event == "stop" and isinstance(loop_count, int) and loop_count > 0:
        emit_note(tool, "Test-convention violations still present (not re-prompting):\n" + "\n".join(violations))

    guidance = (
        "\n\nOnly fix files that are part of your own change. If a listed file is not yours, "
        "report it instead of editing it (AGENTS.md §8)."
        if event == "stop"
        else ""
    )
    emit_block(tool, "Test-convention violation:\n" + "\n".join(violations) + f"\n\n{RULE}" + guidance)


if __name__ == "__main__":
    main()
