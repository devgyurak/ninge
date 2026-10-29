#!/usr/bin/env python3
"""SessionStart hook: put the current phase and the non-negotiables in front of the agent.

Plan §5 gates every phase on its DoD and §8 rule 9 forbids working past the current phase,
so the phase must be known before the first edit. The phase is the single `▶` line in
docs/STATUS.md (scripts/check_agent_config.py fails unless there is exactly one).

Usage:
    session_context.py --tool claude|codex|cursor|opencode|pi
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _hookio import emit_context, hook_root, parse_args

MARKER = "▶"


def current_phase(status_text: str) -> str | None:
    lines = [line.strip() for line in status_text.splitlines() if line.strip().startswith(MARKER)]
    if len(lines) != 1:
        return None
    return lines[0].lstrip(MARKER).strip()


def build_context(root: Path) -> str:
    try:
        status = (root / "docs" / "STATUS.md").read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        status = ""
    phase = current_phase(status) or "unknown — docs/STATUS.md needs exactly one ▶ line"
    return "\n".join(
        (
            "## ninge session context",
            "",
            f"**Current phase:** {phase}",
            "Work inside this phase only; anything later is a stub + `TODO(phaseN)` (AGENTS.md §6).",
            "",
            "**Never:** add a ROM / BIOS / firmware / save or anything extracted from one; open GPL "
            "emulator source; copy or translate GPL code; guess hardware behavior (write it as "
            "Unverified in docs/hw/ + `TODO(hw-unverified)`); put IO, time, randomness, threads, "
            "Rc/RefCell, or `dyn` in ninge-core; change core without a test (AGENTS.md §2, §3).",
            "",
            "**Gate:** `./scripts/verify.sh` — tool hooks are only fast feedback.",
        )
    )


def main() -> None:
    emit_context(parse_args(), build_context(hook_root()))


if __name__ == "__main__":
    main()
