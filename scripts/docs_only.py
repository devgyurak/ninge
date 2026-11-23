#!/usr/bin/env python3
"""Says whether a change touches documentation only, so the gate may skip its cargo steps.

    python3 scripts/docs_only.py <base>     # exit 0: documentation only; exit 1: anything else

`<base>` is a commit. The change is everything between the merge base of `<base>` and `HEAD`
and the working tree, untracked files included, so a local run cannot pass by leaving code
uncommitted. ./scripts/verify.sh calls this when NINGE_DOCS_ONLY_BASE is set, and
.github/workflows/ci.yml calls it to decide which jobs and steps a run needs: one
implementation, and the variable alone never skips anything.

It fails closed. An empty change, a path outside the allow-list, a base git cannot resolve or
any git error is "not documentation only", and the whole gate runs.

Documentation is an allow-list, not "whatever is not code":

  - `docs/`, and the agent settings (`.claude/` `.agents/` `.codex/` `.cursor/` `.opencode/`
    `.pi/`): what the commit convention calls `DOCS:` (AGENTS.md §8);
  - a `*.md` file anywhere outside `crates/`.

Nothing the cargo steps read is on it: no crate includes a Markdown file, and the golden
hashes (`tests/golden/*.txt`), the manifests, `scripts/` and `.github/workflows/` are not
documentation. The Python gates read all of the above and always run.

Standard library only.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# The agent settings are documentation by the commit convention (AGENTS.md §8).
DOC_DIRS = ("docs/", ".claude/", ".agents/", ".codex/", ".cursor/", ".opencode/", ".pi/")
# A crate may come to include a Markdown file (`readme`, `include_str!`), which doc tests then
# read, so nothing under here is documentation.
CODE_DIRS = ("crates/",)


def is_doc(path: str) -> bool:
    """Whether `path` (repository-relative, `/`-separated) is documentation."""
    if path.startswith(CODE_DIRS):
        return False
    return path.startswith(DOC_DIRS) or path.endswith(".md")


def first_non_doc(paths: list[str]) -> str | None:
    """The first path that is not documentation, or `None` when every one is."""
    return next((path for path in paths if not is_doc(path)), None)


def docs_only(paths: list[str]) -> bool:
    """Whether the change is documentation only. No change at all is not."""
    return bool(paths) and first_non_doc(paths) is None


def _git(root: Path, *args: str) -> str | None:
    try:
        result = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=False)
    except OSError:
        return None
    return result.stdout if result.returncode == 0 else None


def changed_paths(base: str, root: Path = ROOT) -> list[str] | None:
    """Every path that differs between the merge base of `base` and `HEAD` and the working tree,
    untracked files included; `None` when git cannot tell."""
    merge_base = _git(root, "merge-base", base, "HEAD")
    if merge_base is None or not merge_base.strip():
        return None
    # --no-renames: a source file moved under docs/ must show as the removal it also is.
    tracked = _git(root, "diff", "--name-only", "--no-renames", "-z", merge_base.strip())
    untracked = _git(root, "ls-files", "--others", "--exclude-standard", "-z")
    if tracked is None or untracked is None:
        return None
    return sorted({path for path in (tracked + untracked).split("\0") if path})


def main(argv: list[str]) -> int:
    if len(argv) != 2 or not argv[1] or argv[1].startswith("-"):
        print("usage: docs_only.py <base-commit>", file=sys.stderr)
        return 1
    paths = changed_paths(argv[1])
    if paths is None:
        print(f"  cannot compare with {argv[1]}: treated as not documentation only")
        return 1
    if not paths:
        print(f"  nothing changed since {argv[1]}: treated as not documentation only")
        return 1
    other = first_non_doc(paths)
    if other is not None:
        print(f"  not documentation only: {other}")
        return 1
    print(f"  documentation only: {len(paths)} path(s) changed since {argv[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
