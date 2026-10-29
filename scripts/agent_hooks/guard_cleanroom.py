#!/usr/bin/env python3
"""Pre-hook (and CI scan) that keeps ninge clean-room and dump-free (AGENTS.md §3).

A mistake here cannot be undone: once a commercial ROM, a BIOS byte, or a translated GPL
function reaches git history, every clone and fork keeps it. So the guard denies, before
the tool runs:

  1. **BIOS / firmware dumps** — any access, anywhere (read, write, pass as an argument).
     ninge boots on HLE BIOS + direct boot + synthetic firmware; an agent that has read a
     BIOS is no longer writing a clean-room HLE.
  2. **ROMs and save files** (.nds .srl .dsi .ids .gba / .sav .dsv) — reading their bytes
     (Read tool, cat/xxd/strings/python…), writing them, or `git` on them, outside the
     directories that hold only our own output (`testroms/`, `tests/fixtures/synthetic/`).
     Running the emulator on a user's ROM (`cargo run -- game.nds`) is allowed.
     `git` on any ROM or save is denied **everywhere**, our own test ROMs included: built
     ROMs are never committed (docs/STATUS.md, "Decided").
  3. **GPL emulator source** — a local checkout (a directory named melonDS / DeSmuME /
     NooDS…) or that project on a code host (GitHub, GitLab, …). Project blogs and notes are
     allowed. Plan §2 lets humans read GPL code for ideas; agents do not, because an agent
     with a GPL function in context while writing core translates it without meaning to.
  4. **GPL text in code** — writing a source file whose content carries a GPL notice.

This guard is **best-effort**: a token/regex heuristic, not a sandbox. When it cannot
decide, it fails closed. `--scan-tracked` applies the same rules to `git ls-files` and is
what ./scripts/verify.sh and CI run.

Usage:
    guard_cleanroom.py --tool claude|codex|cursor|opencode|pi   (hook payload JSON on stdin)
    guard_cleanroom.py --scan-tracked
"""

from __future__ import annotations

import os
import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _hookio import (
    emit_allow,
    emit_deny,
    extract_command,
    extract_paths,
    extract_urls,
    extract_written_text,
    has_flag,
    hook_root,
    looks_like_patch,
    parse_args,
    read_payload,
)

ROM_SUFFIXES = frozenset({".nds", ".srl", ".dsi", ".ids", ".gba"})
SAVE_SUFFIXES = frozenset({".sav", ".dsv"})

# bios7.bin, bios9.bin, biosnds7.rom, biosdsi9.bin, firmware.bin, nand.bin, dsi_nand.img …
DUMP_NAME_RE = re.compile(r"^(?:.*bios.*|.*firmware.*|(?:dsi_?)?nand)\.(?:bin|rom|img)$", re.IGNORECASE)

# GPL emulators agents do not open. AGENTS.md §3.3 states the same list and
# scripts/check_agent_config.py fails if the two differ.
GPL_EMULATORS = frozenset({"melonds", "desmume", "noods"})
_GPL_COMPONENT_RE = re.compile(r"^(?:%s)(?:[-_. ].*)?$" % "|".join(sorted(GPL_EMULATORS)), re.IGNORECASE)
CODE_HOSTS = frozenset(
    {
        "github.com",
        "www.github.com",
        "raw.githubusercontent.com",
        "codeload.github.com",
        "gist.github.com",
        "gitlab.com",
        "codeberg.org",
        "bitbucket.org",
        "sourceforge.net",
        "git.sr.ht",
    }
)

# Directories that hold only our own output, where ROM/save files may legitimately exist.
OWN_OUTPUT_DIRS = (("testroms",), ("tests", "fixtures", "synthetic"))

CODE_SUFFIXES = frozenset({".rs", ".c", ".cc", ".cpp", ".h", ".hpp", ".s", ".asm", ".inc"})
GPL_TEXT_RE = re.compile(r"General Public License|SPDX-License-Identifier:\s*[AL]?GPL", re.IGNORECASE)

# Verbs that read, copy, or execute a file's contents.
FILE_READ_VERBS = frozenset(
    {
        "cat",
        "bat",
        "less",
        "more",
        "head",
        "tail",
        "strings",
        "xxd",
        "hexdump",
        "hd",
        "od",
        "dd",
        "cp",
        "mv",
        "ln",
        "scp",
        "rsync",
        "tar",
        "zip",
        "base64",
        "objdump",
        "python",
        "python3",
        "perl",
        "ruby",
        "node",
        "git",
    }
)
# Shell word-prefixes that are transparently skipped when finding the real command word.
PREFIXES = ("sudo", "time", "env", "nice", "nohup", "command", "exec")
_SEGMENT_SPLIT_RE = re.compile(r"\s*(?:&&|\|\||;|\||\n)\s*")

REASON_DUMP = (
    "Blocked: BIOS and firmware dumps are off-limits to agents (AGENTS.md §3.2). ninge runs on "
    "HLE BIOS + direct boot + synthetic firmware; an agent that reads a dump can no longer "
    "write a clean-room HLE. Work from GBATEK and docs/hw/ instead."
)
REASON_ROM = (
    "Blocked: commercial ROM and save bytes stay out of the agent's context and the repository "
    "(AGENTS.md §3.1). You may run the emulator on a ROM; do not read, copy, write, or commit "
    "one. Our own test ROMs live under testroms/."
)
REASON_GPL = (
    "Blocked: agents do not open GPL emulator source — local checkouts or code-host pages "
    "(AGENTS.md §3.3). Read GBATEK, or a project's blog/notes, and write a prose summary in "
    "docs/hw/. If you need to know how another emulator solved something, ask a human."
)
REASON_COMMIT = (
    "Blocked: no ROM or save file is ever committed, our own built test ROMs included "
    "(AGENTS.md §3.1). Commit the test ROM's source in testroms/; tests build it."
)
REASON_GPL_TEXT = (
    "Blocked: this content carries a GPL notice (AGENTS.md §3.3). ninge does not copy GPL code. "
    "Write the behavior from docs/hw/ in your own words."
)


def _expand(raw: str) -> Path | None:
    try:
        return Path(os.path.expandvars(raw)).expanduser()
    except (ValueError, OSError, RuntimeError):
        return None


def _resolve(path: Path) -> Path:
    # Judge by the target: a testroms/x.nds that is a symlink to ~/roms/game.nds is a ROM read.
    # If resolution fails (NUL byte, symlink loop), fall back to the literal path (fail closed).
    try:
        return path.resolve()
    except (OSError, RuntimeError, ValueError):
        return path


def _in_own_output(resolved: Path, root: Path) -> bool:
    try:
        parts = resolved.relative_to(root).parts
    except ValueError:
        return False
    return any(parts[: len(prefix)] == prefix for prefix in OWN_OUTPUT_DIRS)


def is_dump(raw: str) -> bool:
    path = _expand(raw)
    if path is None:
        return False
    return bool(DUMP_NAME_RE.match(path.name) or DUMP_NAME_RE.match(_resolve(path).name))


def _rom_or_save_suffix(raw: str) -> bool:
    path = _expand(raw)
    return path is not None and path.suffix.lower() in ROM_SUFFIXES | SAVE_SUFFIXES


def is_foreign_rom_or_save(raw: str, root: Path) -> bool:
    path = _expand(raw)
    if path is None:
        return False
    resolved = _resolve(path)
    suffixes = {path.suffix.lower(), resolved.suffix.lower()}
    if not suffixes & (ROM_SUFFIXES | SAVE_SUFFIXES):
        return False
    return not _in_own_output(resolved, root)


def is_gpl_checkout(raw: str) -> bool:
    """True when a path runs through a directory that is a GPL emulator checkout."""
    path = _expand(raw)
    if path is None:
        return False
    for candidate in {path, _resolve(path)}:
        parts = candidate.parts
        # A file merely *named* melonDS.md is fine; a directory component is a checkout.
        dirs = parts[:-1] if not candidate.is_dir() else parts
        if any(_GPL_COMPONENT_RE.match(part) for part in dirs):
            return True
    return False


def is_gpl_url(url: str) -> bool:
    if url.startswith("git@"):
        host, _, rest = url[4:].partition(":")
    else:
        parsed = urlparse(url)
        host, rest = parsed.hostname or "", parsed.path
    if host.lower() not in CODE_HOSTS:
        return False
    return any(_GPL_COMPONENT_RE.match(seg) for seg in rest.split("/") if seg)


def _tokens(segment: str) -> list[str]:
    try:
        toks = shlex.split(segment)
    except ValueError:
        toks = segment.split()
    while toks and (re.match(r"^\w+=", toks[0]) or toks[0] in PREFIXES):
        toks = toks[1:]
    return toks


def _arguments(toks: list[str]) -> list[str]:
    """Argument values, including the value half of `--flag=value`."""
    values: list[str] = []
    for tok in toks[1:]:
        if tok.startswith("-"):
            if "=" in tok:
                values.append(tok.split("=", 1)[1])
            continue
        values.append(tok)
    return values


def blocked_command(command: str, root: Path) -> str | None:
    """The reason a shell command is denied, else None."""
    for segment in _SEGMENT_SPLIT_RE.split(command):
        toks = _tokens(segment)
        if not toks:
            continue
        verb = os.path.basename(toks[0])
        args = _arguments(toks)
        if any(is_dump(arg) for arg in args):
            return REASON_DUMP
        if verb == "git" and any(_rom_or_save_suffix(arg) for arg in args):
            return REASON_COMMIT
        if verb in FILE_READ_VERBS and any(is_foreign_rom_or_save(arg, root) for arg in args):
            return REASON_ROM
        if any(("/" in arg or verb in ("cd", "pushd")) and is_gpl_checkout(arg) for arg in args):
            return REASON_GPL
        # `gh repo clone owner/repo`, `gh api repos/owner/repo/...` name a repo without a URL.
        if verb == "gh" and any("/" in arg and is_gpl_url("https://github.com/" + arg) for arg in args):
            return REASON_GPL
    return None


def judge(payload: dict[str, Any], root: Path) -> str | None:
    """The reason a tool call is denied, else None. Pure over the payload."""
    for raw in extract_paths(payload):
        if is_dump(raw):
            return REASON_DUMP
        if is_foreign_rom_or_save(raw, root):
            return REASON_ROM
        if is_gpl_checkout(raw):
            return REASON_GPL
    for url in extract_urls(payload):
        if is_gpl_url(url):
            return REASON_GPL

    written = extract_written_text(payload)
    if written and GPL_TEXT_RE.search(written):
        targets = extract_paths(payload)
        if any(Path(t).suffix.lower() in CODE_SUFFIXES for t in targets):
            return REASON_GPL_TEXT

    command = extract_command(payload)
    # A Codex apply_patch body is not a shell command: its paths were checked above.
    if command and not looks_like_patch(command):
        return blocked_command(command, root)
    return None


def scan_tracked(root: Path) -> list[str]:
    """Tracked files that break §3. Returns one line per finding."""
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0:
        return [f"  git ls-files failed: {result.stderr.decode('utf-8', 'replace').strip()}"]
    findings: list[str] = []
    for rel in (p for p in result.stdout.decode("utf-8", "surrogateescape").split("\0") if p):
        path = root / rel
        if DUMP_NAME_RE.match(path.name):
            findings.append(f"  {rel} — a BIOS/firmware dump name (§3.2)")
        elif path.suffix.lower() in ROM_SUFFIXES | SAVE_SUFFIXES:
            # No exception for testroms/: built test ROMs are never committed.
            findings.append(f"  {rel} — a ROM/save file is tracked (§3.1)")
        if any(_GPL_COMPONENT_RE.match(part) for part in Path(rel).parts[:-1]):
            findings.append(f"  {rel} — inside a directory named like a GPL emulator checkout (§3.3)")
        if path.suffix.lower() in CODE_SUFFIXES and path.is_file():
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if GPL_TEXT_RE.search(text):
                findings.append(f"  {rel} — carries a GPL notice (§3.3)")
    return findings


def main() -> None:
    root = hook_root()
    if has_flag("--scan-tracked"):
        findings = scan_tracked(root)
        if findings:
            print("clean-room scan FAILED:\n" + "\n".join(findings), file=sys.stderr)
            raise SystemExit(1)
        print("clean-room scan: all checks passed")
        raise SystemExit(0)

    tool = parse_args()
    payload = read_payload()
    try:
        reason = judge(payload, root)
    except Exception as exc:
        emit_deny(tool, f"Blocked: clean-room guard error, failing closed: {exc!r}", payload)
        return
    if reason:
        emit_deny(tool, reason, payload)
    emit_allow(tool)


if __name__ == "__main__":
    main()
