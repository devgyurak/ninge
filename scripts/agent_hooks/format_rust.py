#!/usr/bin/env python3
"""Post-hook that applies `rustfmt` to just-edited Rust files.

The repository is kept `cargo fmt`-clean, so formatting an edited file leaves a diff only
where the file actually changed. rustfmt picks up `rustfmt.toml` from the file's parents;
`--edition 2024` matches the workspace edition (plan §3.1) for files rustfmt would otherwise
read as edition 2015. When rustfmt is not installed (or no Cargo workspace exists yet) the
hook is a silent no-op — `cargo fmt --all --check` in ./scripts/verify.sh is the real gate.

Usage:
    format_rust.py --tool claude|codex|cursor|opencode|pi
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _hookio import emit_allow, emit_note, extract_paths, hook_root, parse_args, read_payload

EDITION = "2024"


def _eligible(raw: str, root: Path) -> str | None:
    """Return the repo-relative path when the file is a formattable target, else None."""
    path = Path(raw).expanduser()
    if path.suffix != ".rs" or not path.is_file():
        return None
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError:
        return None  # outside the repository


def main() -> None:
    tool = parse_args()
    payload = read_payload()
    root = hook_root()

    targets = [rel for rel in (_eligible(p, root) for p in extract_paths(payload)) if rel]
    rustfmt = shutil.which("rustfmt")
    if not targets or rustfmt is None:
        emit_allow(tool)

    try:
        result = subprocess.run(
            [str(rustfmt), "--edition", EDITION, *targets],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        emit_allow(tool)
        return

    names = ", ".join(Path(t).name for t in targets)
    if result.returncode != 0:
        # A parse error: tell the agent now instead of at the verify gate. Do not block —
        # the agent is mid-edit.
        emit_note(tool, f"rustfmt could not format {names}:\n{result.stderr.strip()[:2000]}")
    emit_note(tool, f"rustfmt applied: {names}")


if __name__ == "__main__":
    main()
