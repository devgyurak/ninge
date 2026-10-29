#!/usr/bin/env python3
"""Post-hook (and CI gate) for ninge's architecture rules (AGENTS.md §2, §3.5).

Rust sources:
  * `crates/core/src/**` is the pure core. No IO, clocks, randomness, threads, printing,
    shared or interior mutability, hash containers with a per-process seed, `dyn`, or raw
    pointers. One finding can be waived with `// PURITY-OK: <reason>` on the line or the
    line above; the reviewer has to agree with the reason.
  * every `unsafe` anywhere in the workspace carries a `SAFETY:` comment on the line or in
    the three lines above (AGENTS.md §2 rule 10 — the measurement goes in the PR);
  * every source file under `crates/`, `testroms/`, and `tests/` starts with the SPDX line
    `SPDX-License-Identifier: Apache-2.0` (AGENTS.md §3.7).

Cargo manifests:
  * `ninge-core` depends on no workspace crate and on none of CORE_FORBIDDEN_CRATES;
  * nothing depends on the frontend package `ninge`; `ninge-runner` does not depend on
    `ninge-tracediff`;
  * package, lib, and bin names are `ninge` / `ninge-*` and carry no trademark token;
  * every package declares `license = "Apache-2.0"` or `license.workspace = true`, and a
    `[workspace.package]` license is `Apache-2.0`.

Modes: default (paths from the hook payload), `--event stop` (changed files from git, for
Cursor), `--all` (the whole repository; exit non-zero on a finding — what CI runs).

The decision logic lives here only. `scripts/check_agent_config.py` imports
CORE_FORBIDDEN_CRATES to prove AGENTS.md states the same list (one list, no drift).
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
from _rustscan import line_of, strip

CORE_PACKAGE = "ninge-core"
# The frontend package is plain `ninge` so that `cargo install ninge` installs the binary.
FRONTEND_PACKAGE = "ninge"
RUNNER_PACKAGE = "ninge-runner"
TRACEDIFF_PACKAGE = "ninge-tracediff"

# Crates ninge-core must not depend on: terminal / audio / gamepad IO, threads, randomness,
# wall-clock time, OS access, and the binary-side error type. AGENTS.md §2 rule 1 states the
# same list and scripts/check_agent_config.py fails if the two differ.
# `rayon` is here on purpose: plan §3.2 names it for the Phase 7 rasterizer, but plan §8
# rule 3 bans thread dependencies in core. Until a `decision` issue settles it, the stricter rule holds.
CORE_FORBIDDEN_CRATES = frozenset(
    {
        "anyhow",
        "base64",
        "chrono",
        "cpal",
        "crossbeam",
        "crossterm",
        "fastrand",
        "flate2",
        "getrandom",
        "gilrs",
        "libc",
        "memmap2",
        "mio",
        "nix",
        "parking_lot",
        "rand",
        "ratatui",
        "rayon",
        "time",
        "tokio",
    }
)

# Who may not depend on whom (besides the core rule above).
FORBIDDEN_EDGES = {
    CORE_PACKAGE: frozenset({FRONTEND_PACKAGE, RUNNER_PACKAGE, TRACEDIFF_PACKAGE}),
    RUNNER_PACKAGE: frozenset({FRONTEND_PACKAGE, TRACEDIFF_PACKAGE}),
    TRACEDIFF_PACKAGE: frozenset({FRONTEND_PACKAGE}),
}

# Trademark tokens that must not appear in a crate, lib, or binary name (plan §2, §8 rule 10).
TRADEMARK_TOKENS = frozenset({"nintendo", "pokemon", "pokémon", "nds", "ds", "dsi", "gba", "gameboy"})
NAME_EXEMPT = frozenset({"xtask"})

LICENSE = "Apache-2.0"
SPDX_LINE = f"SPDX-License-Identifier: {LICENSE}"
# Sources that carry the SPDX header: Rust under crates/ and tests/, and test-ROM sources.
SPDX_ROOTS = ("crates/", "testroms/", "tests/")
SPDX_SUFFIXES = frozenset({".rs", ".s", ".S", ".c", ".h"})

# (pattern over stripped code, what it is, why core cannot have it)
CORE_FORBIDDEN_CODE: tuple[tuple[re.Pattern[str], str], ...] = (
    (re.compile(r"\bstd::(?:fs|net|process|thread|env|io)\b"), "std IO / process / thread / env access"),
    (re.compile(r"\b(?:SystemTime|Instant|UNIX_EPOCH)\b"), "a wall or monotonic clock"),
    (re.compile(r"\b(?:print|println|eprint|eprintln|dbg)!"), "printing to stdout/stderr"),
    (re.compile(r"\b(?:Rc|Weak|RefCell|Cell|UnsafeCell|OnceCell)\b"), "shared or interior mutability"),
    (
        re.compile(r"\b(?:Arc|Mutex|RwLock|Condvar|Barrier|LazyLock|OnceLock|thread_local)\b|\bAtomic[A-Z]\w*"),
        "thread-shared state",
    ),
    (re.compile(r"\bstatic\s+mut\b"), "global mutable state"),
    (re.compile(r"\b(?:HashMap|HashSet|RandomState)\b"), "a hash container with a per-process random seed"),
    (re.compile(r"\bdyn\b"), "dynamic dispatch (trait objects, boxed closures)"),
    (re.compile(r"\*\s*(?:const|mut)\b"), "a raw pointer"),
)
# `use std::{fs, io}` would slip past `std::fs`.
_STD_GROUP_RE = re.compile(r"\bstd::\{([^}]*)\}")
_STD_GROUP_BANNED = frozenset({"fs", "net", "process", "thread", "env", "io"})
_UNSAFE_RE = re.compile(r"\bunsafe\b")

WHY_CORE = (
    "ninge-core is IO-free, single-owner, and deterministic (plan §4.2, §8 rules 3-5): its only "
    "inputs are button/touch state and its only outputs are two frame buffers and audio "
    "samples; every piece of state is plain serde data owned by the top-level struct and "
    "reached through &mut self. Move IO to ninge-runner / the frontend (`ninge`), keep collections "
    "to arrays, Vec, or BTreeMap, and keep dispatch static (generics)."
)
WHY_UNSAFE = "`unsafe` needs a `// SAFETY:` comment, and the PR cites the measurement that justified it (plan §8 rule 6)."
WHY_LICENSE = (
    f"ninge is {LICENSE} (AGENTS.md §3.7): line 1 of every source file is a comment carrying "
    f"`{SPDX_LINE}`, and every package declares the license."
)
WHY_MANIFEST = (
    "Crate edges point at ninge-core, never away from it (AGENTS.md §2). Names are ninge / "
    "ninge-* and carry no trademark (plan §2, §8 rule 10)."
)


def _rel(path: Path, root: Path) -> str | None:
    try:
        return path.resolve().relative_to(root).as_posix()
    except (ValueError, OSError):
        return None


def _waived(lines: list[str], line: int, marker: str) -> bool:
    here = lines[line - 1] if 0 < line <= len(lines) else ""
    above = lines[line - 2] if 1 < line <= len(lines) + 1 else ""
    return marker in here or marker in above


def check_rust_source(source: str, label: str, *, core: bool) -> list[str]:
    """Violations in one Rust source text. Pure, so the selftest can feed it text."""
    stripped = strip(source)
    lines = source.splitlines()
    violations: list[str] = []

    if core:
        for pattern, what in CORE_FORBIDDEN_CODE:
            for match in pattern.finditer(stripped):
                line = line_of(stripped, match.start())
                if not _waived(lines, line, "PURITY-OK:"):
                    violations.append(f"  {label}:{line} — `{match.group(0).strip()}` in ninge-core: {what}.")
        for group in _STD_GROUP_RE.finditer(stripped):
            names = {part.strip().split("::", 1)[0] for part in group.group(1).split(",")}
            line = line_of(stripped, group.start())
            banned = sorted(names & _STD_GROUP_BANNED)
            if banned and not _waived(lines, line, "PURITY-OK:"):
                violations.append(f"  {label}:{line} — `std::{{{', '.join(banned)}}}` in ninge-core: std IO access.")

    for match in _UNSAFE_RE.finditer(stripped):
        line = line_of(stripped, match.start())
        context = lines[max(0, line - 4) : line]
        if not any("SAFETY:" in text or "# Safety" in text for text in context):
            violations.append(f"  {label}:{line} — `unsafe` without a `// SAFETY:` comment.")
    return violations


# --- Cargo.toml (a small line parser: hooks must run without tomllib on Python 3.9) ------

_HEADER_RE = re.compile(r"^\s*\[\[?\s*([^\]]+?)\s*\]\]?\s*(?:#.*)?$")
_DEP_TABLE_RE = re.compile(
    r"""^(?:target\.(?:'[^']*'|"[^"]*"|[^.]+)\.)?((?:dev-|build-)?dependencies)(?:\.(.+))?$"""
)
_KEY_RE = re.compile(r'^\s*("([^"]+)"|[A-Za-z0-9_\-]+)\s*=\s*(.*)$')
_STR_VALUE_RE = re.compile(r"""^\s*["']([^"']*)["']""")
_PACKAGE_RE = re.compile(r"""\bpackage\s*=\s*["']([^"']+)["']""")
_WORKSPACE_TRUE_RE = re.compile(r"\bworkspace\s*=\s*true\b")
_LICENSE_RE = re.compile(r"^\s*license(\.workspace)?\s*=\s*(.*)$")
_TRUE_RE = re.compile(r"^\s*true\b")


class Manifest:
    def __init__(self) -> None:
        self.package: str | None = None
        self.lib: str | None = None
        self.bins: list[str] = []
        # (dependency kind, real package name, line, inherited from the workspace)
        self.deps: list[tuple[str, str, int, bool]] = []
        # [workspace.dependencies] key -> real package name
        self.workspace_deps: dict[str, str] = {}
        # "Apache-2.0", "workspace" (license.workspace = true), or None when absent
        self.license: str | None = None
        self.workspace_license: str | None = None


def parse_manifest(text: str) -> Manifest:
    manifest = Manifest()
    table = ""
    pending: tuple[str, str, int] | None = None  # a [dependencies.foo] table being read

    def close_pending() -> None:
        nonlocal pending
        if pending is not None:
            kind, name, line = pending
            manifest.deps.append((kind, name, line, False))
            pending = None

    for lineno, raw in enumerate(text.splitlines(), start=1):
        header = _HEADER_RE.match(raw)
        if header:
            close_pending()
            table = header.group(1).strip()
            dep_table = _DEP_TABLE_RE.match(table)
            if dep_table and dep_table.group(2):
                pending = (dep_table.group(1), dep_table.group(2).strip().strip("\"'"), lineno)
            if table == "bin":
                manifest.bins.append("")
            continue
        license_key = _LICENSE_RE.match(raw)
        if license_key and table in ("package", "workspace.package") and pending is None:
            value = license_key.group(2)
            if license_key.group(1):
                found = "workspace" if _TRUE_RE.match(value) else value.strip()
            else:
                literal = _STR_VALUE_RE.match(value)
                found = literal.group(1) if literal else value.strip()
            if table == "package":
                manifest.license = found
            else:
                manifest.workspace_license = found
            continue
        key = _KEY_RE.match(raw)
        if key is None:
            continue
        name = key.group(2) or key.group(1)
        value = key.group(3)
        if pending is not None:
            if name == "package":
                literal = _STR_VALUE_RE.match(value)
                if literal:
                    pending = (pending[0], literal.group(1), pending[2])
            continue
        if table in ("package", "lib") and name == "name":
            literal = _STR_VALUE_RE.match(value)
            if literal:
                if table == "package":
                    manifest.package = literal.group(1)
                else:
                    manifest.lib = literal.group(1)
        elif table == "bin" and name == "name" and manifest.bins:
            literal = _STR_VALUE_RE.match(value)
            if literal:
                manifest.bins[-1] = literal.group(1)
        elif table == "workspace.dependencies":
            renamed = _PACKAGE_RE.search(value)
            manifest.workspace_deps[name] = renamed.group(1) if renamed else name
        else:
            dep_table = _DEP_TABLE_RE.match(table)
            if dep_table and not dep_table.group(2):
                renamed = _PACKAGE_RE.search(value)
                inherited = bool(_WORKSPACE_TRUE_RE.search(value))
                manifest.deps.append((dep_table.group(1), renamed.group(1) if renamed else name, lineno, inherited))
    close_pending()
    manifest.bins = [b for b in manifest.bins if b]
    return manifest


def _trademark_tokens(name: str) -> list[str]:
    lowered = name.lower()
    tokens = set(re.split(r"[-_]", lowered))
    hits = tokens & TRADEMARK_TOKENS
    hits |= {word for word in ("nintendo", "pokemon", "pokémon") if word in lowered}
    return sorted(hits)


def check_manifest_text(text: str, label: str, workspace_deps: dict[str, str] | None = None) -> list[str]:
    """Violations in one Cargo.toml. Pure, so the selftest can feed it text."""
    manifest = parse_manifest(text)
    renames = dict(workspace_deps or {})
    renames.update(manifest.workspace_deps)
    violations: list[str] = []

    for kind, name in (
        ("package", manifest.package),
        ("lib", manifest.lib),
        *(("bin", b) for b in manifest.bins),
    ):
        if not name or name in NAME_EXEMPT:
            continue
        if not (name == "ninge" or re.match(r"^ninge[-_]", name)):
            violations.append(f"  {label} — {kind} name `{name}` is not `ninge` / `ninge-*`.")
        hits = _trademark_tokens(name)
        if hits:
            violations.append(f"  {label} — {kind} name `{name}` carries a trademark token: {', '.join(hits)}.")

    if manifest.workspace_license is not None and manifest.workspace_license != LICENSE:
        violations.append(f"  {label} — [workspace.package] license is `{manifest.workspace_license}`, not `{LICENSE}`.")
    package = manifest.package
    if package is None:
        return violations
    if package not in NAME_EXEMPT and manifest.license not in (LICENSE, "workspace"):
        found = manifest.license or "missing"
        violations.append(f"  {label} — `{package}` license is {found}; declare `license = \"{LICENSE}\"` or `license.workspace = true`.")
    for kind, dep, line, inherited in manifest.deps:
        real = renames.get(dep, dep) if inherited else dep
        if package == CORE_PACKAGE and kind != "build-dependencies":
            if kind == "dependencies" and real in CORE_FORBIDDEN_CRATES:
                violations.append(f"  {label}:{line} — ninge-core depends on `{real}` (forbidden in core).")
        if real in FORBIDDEN_EDGES.get(package, frozenset()):
            violations.append(f"  {label}:{line} — `{package}` depends on `{real}` ({kind}).")
        if real == FRONTEND_PACKAGE and package != FRONTEND_PACKAGE:
            violations.append(f"  {label}:{line} — `{package}` depends on the frontend (`ninge`); nothing may.")
    return violations


def _workspace_deps(root: Path) -> dict[str, str]:
    top = root / "Cargo.toml"
    if not top.is_file():
        return {}
    try:
        return parse_manifest(top.read_text(encoding="utf-8")).workspace_deps
    except (OSError, UnicodeDecodeError):
        return {}


def check_file(path: Path) -> list[str]:
    root = hook_root()
    if not path.is_file():
        return []
    rel = _rel(path, root)
    if rel is None:
        return []
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    if path.name == "Cargo.toml":
        return check_manifest_text(text, rel, _workspace_deps(root))
    violations = []
    if path.suffix == ".rs":
        violations = check_rust_source(text, rel, core=rel.startswith("crates/core/src/"))
    return violations + check_spdx(text, rel)


def check_spdx(text: str, rel: str) -> list[str]:
    """The SPDX header on line 1 of a source under crates/ or testroms/. Pure."""
    if not rel.startswith(SPDX_ROOTS) or Path(rel).suffix not in SPDX_SUFFIXES:
        return []
    first = text.split("\n", 1)[0]
    if SPDX_LINE in first:
        return []
    return [f"  {rel}:1 — line 1 must be a comment carrying `{SPDX_LINE}`."]


def _explain(violations: list[str]) -> str:
    notes = []
    if any("ninge-core:" in v for v in violations):
        notes.append(f"- {WHY_CORE}")
    if any("`unsafe`" in v for v in violations):
        notes.append(f"- {WHY_UNSAFE}")
    if any("SPDX" in v or "license" in v for v in violations):
        notes.append(f"- {WHY_LICENSE}")
    if any("Cargo.toml" in v and "license" not in v for v in violations):
        notes.append(f"- {WHY_MANIFEST}")
    return "\n".join(notes)


def main() -> None:
    tool = parse_args()
    root = hook_root()

    if has_flag("--all"):
        files = [*repo_files(root, "Cargo.toml"), *(f for x in sorted(SPDX_SUFFIXES) for f in repo_files(root, x))]
        violations = [v for path in files for v in check_file(path)]
        if violations:
            print("architecture check FAILED:\n" + "\n".join(violations) + "\n\n" + _explain(violations), file=sys.stderr)
            raise SystemExit(1)
        print("architecture check: all checks passed")
        raise SystemExit(0)

    payload = read_payload()
    event = parse_option("--event")
    paths = changed_paths(root) if event == "stop" else extract_paths(payload)
    violations = [v for raw in paths for v in check_file(Path(raw).expanduser())]
    if not violations:
        emit_allow(tool)

    loop_count = payload.get("loop_count")
    if event == "stop" and isinstance(loop_count, int) and loop_count > 0:
        emit_note(tool, "Architecture violations still present (not re-prompting):\n" + "\n".join(violations))

    guidance = (
        "\n\nOnly fix files that are part of your own change. If a listed file is not yours, "
        "report it instead of editing it (AGENTS.md §8)."
        if event == "stop"
        else ""
    )
    emit_block(
        tool,
        "Architecture violation (AGENTS.md §2):\n" + "\n".join(violations) + "\n\n" + _explain(violations) + guidance,
    )


if __name__ == "__main__":
    main()
