#!/usr/bin/env python3
"""Post-hook (and CI gate) for the English-only rule (AGENTS.md §9).

Everything in the repository is English. The only exception is an approved translation of a
document: `<name>-<code>.md` (`README-KO.md`, `docs/PLAN-KO.md`, `README-zh-CN.md`), which
must sit next to its English original `<name>.md`.

The check flags characters from non-Latin scripts (Hangul, CJK, kana, Cyrillic, Arabic, …)
in any text file outside a translation. Latin with diacritics (é), Greek (μ), arrows, and
box-drawing characters are fine. It cannot tell good English from bad — it only catches the
wrong language. A non-Latin character a test genuinely needs is written as an escape
(`'\\u{AC00}'`), so the source stays English.

Modes: default (paths from the hook payload), `--event stop` (changed files from git, for
Cursor), `--all` (every text file in the repository; exit non-zero on a finding).
"""

from __future__ import annotations

import re
import sys
import unicodedata
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

TEXT_SUFFIXES = frozenset(
    {
        ".md",
        ".mdc",
        ".txt",
        ".rs",
        ".py",
        ".toml",
        ".json",
        ".yml",
        ".yaml",
        ".js",
        ".mjs",
        ".ts",
        ".sh",
        ".s",
        ".S",
        ".c",
        ".h",
        ".ld",
        ".snap",
    }
)
TEXT_NAMES = frozenset({".gitignore", ".gitattributes", "Makefile", "CODEOWNERS"})

# `<name>-<code>.md`: a two-letter language code, optionally with a region or script.
TRANSLATION_RE = re.compile(r"^(?P<base>.+)-(?P<code>[A-Za-z]{2}(?:-[A-Za-z]{2,4})?)\.md$")

# Unicode name prefixes of scripts that are not English text.
FOREIGN_SCRIPTS = (
    "HANGUL",
    "CJK",
    "IDEOGRAPHIC",
    "HIRAGANA",
    "KATAKANA",
    "BOPOMOFO",
    "FULLWIDTH",
    "HALFWIDTH",
    "CYRILLIC",
    "ARABIC",
    "HEBREW",
    "THAI",
    "DEVANAGARI",
    "BENGALI",
    "TAMIL",
    "GEORGIAN",
    "ARMENIAN",
    "ETHIOPIC",
    "KHMER",
    "LAO",
    "MYANMAR",
)

RULE = (
    "English only (AGENTS.md §9): code, comments, docs, strings, and test names are English. "
    "A translated document is `<name>-<code>.md` next to its English `<name>.md`, and exists "
    "only when a human approved it. A non-Latin character a test needs is written as an escape."
)


def is_translation(path: Path) -> bool:
    return bool(TRANSLATION_RE.match(path.name))


def foreign_characters(text: str) -> list[tuple[int, int, str]]:
    """(line, column, character) of every character from a non-Latin script. Pure."""
    if text.isascii():
        return []
    found: list[tuple[int, int, str]] = []
    for lineno, line in enumerate(text.splitlines(), start=1):
        if line.isascii():
            continue
        for col, ch in enumerate(line, start=1):
            if ord(ch) > 127 and unicodedata.name(ch, "").startswith(FOREIGN_SCRIPTS):
                found.append((lineno, col, ch))
    return found


def check_file(path: Path) -> list[str]:
    root = hook_root()
    if not path.is_file():
        return []
    try:
        rel = path.resolve().relative_to(root).as_posix()
    except ValueError:
        return []
    if is_translation(path):
        match = TRANSLATION_RE.match(path.name)
        original = path.with_name(match.group("base") + ".md") if match else path
        if not original.is_file():
            return [f"  {rel} — a translation needs its English original `{original.name}` beside it."]
        return []
    if path.suffix not in TEXT_SUFFIXES and path.name not in TEXT_NAMES:
        return []
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    hits = foreign_characters(text)
    if not hits:
        return []
    lines = sorted({line for line, _, _ in hits})
    shown = ", ".join(str(n) for n in lines[:10]) + (" …" if len(lines) > 10 else "")
    sample = "".join(ch for _, _, ch in hits[:12])
    return [f"  {rel} — non-English text on line(s) {shown} (e.g. {sample!r})."]


def _all_text_files(root: Path) -> list[Path]:
    seen: set[Path] = set()
    for suffix in (*TEXT_SUFFIXES, *TEXT_NAMES):
        seen.update(repo_files(root, suffix))
    return sorted(seen)


def main() -> None:
    tool = parse_args()
    root = hook_root()

    if has_flag("--all"):
        violations = [v for path in _all_text_files(root) for v in check_file(path)]
        if violations:
            print("language check FAILED:\n" + "\n".join(violations) + f"\n\n{RULE}", file=sys.stderr)
            raise SystemExit(1)
        print("language check: all checks passed")
        raise SystemExit(0)

    payload = read_payload()
    event = parse_option("--event")
    paths = changed_paths(root) if event == "stop" else extract_paths(payload)
    violations = [v for raw in paths for v in check_file(Path(raw).expanduser())]
    if not violations:
        emit_allow(tool)

    loop_count = payload.get("loop_count")
    if event == "stop" and isinstance(loop_count, int) and loop_count > 0:
        emit_note(tool, "Language violations still present (not re-prompting):\n" + "\n".join(violations))

    guidance = (
        "\n\nOnly fix files that are part of your own change. If a listed file is not yours, "
        "report it instead of editing it (AGENTS.md §8)."
        if event == "stop"
        else ""
    )
    emit_block(tool, "Language violation:\n" + "\n".join(violations) + f"\n\n{RULE}" + guidance)


if __name__ == "__main__":
    main()
