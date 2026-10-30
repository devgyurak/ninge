#!/usr/bin/env python3
"""Self-test for the ninge agent hooks and the agent-config checker.

CI (`.github/workflows/agents.yml`) and ./scripts/verify.sh run this; run it locally with:

    python3 scripts/agent_hooks/selftest.py

Each case feeds a hook payload on stdin (or a staged temp tree) and asserts the decision.
Standard library only, no network, no writes outside a temp tree.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO_ROOT / "scripts"))

FAILURES: list[str] = []


def _run(
    script: str,
    tool: str,
    payload: dict[str, object] | None,
    *,
    root: Path | None = None,
    args: tuple[str, ...] = (),
    python: str = sys.executable,
) -> subprocess.CompletedProcess[str]:
    env = dict(os.environ)
    env["PYTHONDONTWRITEBYTECODE"] = "1"  # keep the hook dir free of __pycache__
    if root is not None:
        env["NINGE_HOOK_ROOT"] = str(root)
    return subprocess.run(
        [python, str(HERE / script), "--tool", tool, *args],
        input=json.dumps(payload) if payload is not None else "",
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
        env=env,
        cwd=str(root or REPO_ROOT),
    )


def _checked(name: str, condition: bool, detail: str = "") -> None:
    if not condition:
        FAILURES.append(f"{name}{': ' + detail.strip()[:400] if detail else ''}")


def _denied(result: subprocess.CompletedProcess[str]) -> bool:
    return "permissionDecision" in result.stdout and "deny" in result.stdout


def _blocked(result: subprocess.CompletedProcess[str]) -> bool:
    return '"decision": "block"' in result.stdout


def _write(root: Path, rel: str, text: str = "") -> Path:
    target = root / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding="utf-8")
    return target


def _read(path: str) -> dict[str, object]:
    return {"tool_input": {"file_path": path}}


def _bash(command: str) -> dict[str, object]:
    return {"tool_input": {"command": command}}


def _fetch(url: str) -> dict[str, object]:
    return {"tool_name": "WebFetch", "tool_input": {"url": url, "prompt": "x"}}


# --- guard_cleanroom ---------------------------------------------------------------------


def test_guard_cleanroom() -> None:
    with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as elsewhere:
        root = Path(tmp).resolve()
        own_rom = _write(root, "testroms/build/arm9_first_pixel.nds")
        _write(root, "docs/hw/melonds-blog-notes.md", "notes")
        checkout = Path(elsewhere).resolve() / "melonDS"
        (checkout / "src").mkdir(parents=True)
        (root / "testroms" / "linked.nds").symlink_to(Path.home() / "roms" / "game.nds")
        allowed = (
            ("docs read", _read(str(REPO_ROOT / "AGENTS.md"))),
            ("own test ROM", _read(str(own_rom))),
            ("note named after an emulator", _read(str(root / "docs/hw/melonds-blog-notes.md"))),
            ("run the emulator on a ROM", _bash("cargo run -p ninge -- ~/roms/game.nds")),
            ("run the built binary", _bash("./target/release/ninge ~/roms/game.nds")),
            ("grep the repo for a name", _bash('rg -n "melonDS" docs')),
            ("emulator blog", _fetch("https://melonds.kuribo64.net/comments.php?id=56")),
            ("GBATEK", _fetch("https://problemkaputt.de/gbatek.htm")),
            ("benign python", _bash('python3 -c "print(1)"')),
            (
                "ordinary core edit",
                {"tool_input": {"file_path": str(root / "crates/core/src/cpu.rs"), "content": "pub fn f() {}"}},
            ),
        )
        denied = (
            ("BIOS read", _read(str(Path.home() / "nds" / "bios7.bin"))),
            ("firmware read", _read(str(root / "firmware.bin"))),
            ("BIOS as an emulator flag", _bash("cargo run -p ninge-runner -- --bios9=~/nds/biosnds9.rom")),
            ("BIOS as an emulator arg", _bash("./target/debug/ninge --bios7 ~/nds/bios7.bin game.nds")),
            ("commercial ROM read", _read(str(Path.home() / "roms" / "game.nds"))),
            ("ROM hexdump", _bash("xxd ~/roms/game.nds | head")),
            ("ROM copied in", _bash("cp ~/roms/game.nds testroms/")),
            ("save committed", _bash("git add saves/game.sav")),
            ("own built test ROM committed", _bash("git add -f testroms/build/arm9_first_pixel.nds")),
            ("save read", _read("/tmp/game.dsv")),
            ("ROM symlinked into testroms", _read(str(root / "testroms" / "linked.nds"))),
            ("GPL checkout read", _read(str(checkout / "src" / "ARM.cpp"))),
            ("GPL checkout grep", {"tool_input": {"pattern": "LDM", "path": str(checkout)}}),
            ("GPL checkout via shell", _bash(f"cat {checkout}/src/ARM.cpp")),
            ("GPL code page", _fetch("https://github.com/melonDS-emu/melonDS/blob/master/src/ARM.cpp")),
            ("GPL raw file", _fetch("https://raw.githubusercontent.com/TASEmulators/desmume/master/x.cpp")),
            ("GPL clone", _bash("git clone https://github.com/Hydr8gon/NooDS")),
            ("GPL clone over ssh", _bash("git clone git@github.com:melonDS-emu/melonDS.git")),
            ("GPL gh clone", _bash("gh repo clone melonDS-emu/melonDS")),
            (
                "GPL notice written into core",
                {
                    "tool_input": {
                        "file_path": str(root / "crates/core/src/arm.rs"),
                        "content": "// This program is free software: GNU General Public License v3\n",
                    }
                },
            ),
            ("NUL path fails closed", _read("a\u0000b.nds")),
        )
        for name, payload in allowed:
            result = _run("guard_cleanroom.py", "claude", payload, root=root)
            _checked(f"guard: allow {name}", not _denied(result) and result.returncode == 0, result.stdout)
        for name, payload in denied:
            result = _run("guard_cleanroom.py", "claude", payload, root=root)
            _checked(f"guard: deny {name}", _denied(result), result.stdout or result.stderr)

        # OpenCode / Pi convention: deny = exit 2 + stderr reason.
        oc = _run("guard_cleanroom.py", "opencode", _read("/tmp/bios7.bin"), root=root)
        _checked("guard: opencode exit 2", oc.returncode == 2 and "Blocked" in oc.stderr, f"rc={oc.returncode}")
        # Cursor convention: deny = permission JSON.
        cur = _run("guard_cleanroom.py", "cursor", _read("/tmp/bios7.bin"), root=root)
        _checked("guard: cursor deny", json.loads(cur.stdout or "{}").get("permission") == "deny", cur.stdout)
        # Malformed input must not crash the hook into an allow.
        weird = _run("guard_cleanroom.py", "claude", _read("x\u0000y"), root=root)
        _checked("guard: no crash on NUL path", "Traceback" not in weird.stderr, weird.stderr)


def test_guard_scan_tracked() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        _write(root, "crates/core/src/lib.rs", "pub fn f() {}\n")
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
        clean = _run("guard_cleanroom.py", "claude", None, root=root, args=("--scan-tracked",))
        _checked("scan: clean tree passes", clean.returncode == 0, clean.stderr)

        _write(root, "tests/fixtures/game.nds", "x")
        _write(root, "bios9.bin", "x")
        _write(root, "src/gpl.c", "/* GNU General Public License */\n")
        subprocess.run(["git", "-C", str(root), "add", "-A", "-f"], check=True)
        dirty = _run("guard_cleanroom.py", "claude", None, root=root, args=("--scan-tracked",))
        _checked(
            "scan: tracked ROM, dump, and GPL notice fail",
            dirty.returncode == 1
            and "game.nds" in dirty.stderr
            and "bios9.bin" in dirty.stderr
            and "gpl.c" in dirty.stderr,
            dirty.stderr,
        )


# --- check_architecture -----------------------------------------------------------------


SPDX = "// SPDX-License-Identifier: Apache-2.0\n"

CORE_OK = """\
//! The core never prints \"std::fs\" — strings and comments do not count: std::fs, dyn, Rc.
use std::collections::BTreeMap;

pub struct Machine {
    pub ram: Vec<u8>,
    pub events: BTreeMap<u64, u32>,
}

impl Machine {
    pub fn step(&mut self) -> u32 {
        let label = 'x';
        let _ = label;
        self.ram.len() as u32
    }
}
"""

CORE_BAD = (
    ("std::fs", "pub fn f() { let _ = std::fs::read(\"x\"); }\n"),
    ("grouped std import", "use std::{collections::BTreeMap, fs};\n"),
    ("Instant", "use std::time::Instant;\n"),
    ("println", 'pub fn f() { println!("hi"); }\n'),
    ("Rc", "use std::rc::Rc;\npub struct S { a: Rc<u8> }\n"),
    ("RefCell", "pub struct S { a: core::cell::RefCell<u8> }\n"),
    ("Arc", "pub struct S { a: std::sync::Arc<u8> }\n"),
    ("HashMap", "pub struct S { a: std::collections::HashMap<u8, u8> }\n"),
    ("dyn", "pub struct S { hook: Box<dyn Fn()> }\n"),
    ("raw pointer", "pub struct S { page: *const u8 }\n"),
    ("static mut", "static mut COUNTER: u32 = 0;\n"),
)


def test_check_architecture_sources() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()

        def run(rel: str, text: str, *, header: bool = True) -> subprocess.CompletedProcess[str]:
            path = _write(root, rel, (SPDX + text) if header else text)
            return _run("check_architecture.py", "claude", _read(str(path)), root=root)

        ok = run("crates/core/src/machine.rs", CORE_OK)
        _checked("arch: allow pure core", not _blocked(ok), ok.stdout)
        for index, (name, text) in enumerate(CORE_BAD):
            result = run(f"crates/core/src/case{index}.rs", text)
            _checked(f"arch: block core {name}", _blocked(result), result.stdout)

        waived = run("crates/core/src/debug.rs", "// PURITY-OK: debugger callback, off the hot path\npub type Hook = Box<dyn Fn(u32)>;\n")
        _checked("arch: allow a PURITY-OK waiver", not _blocked(waived), waived.stdout)
        runner = run("crates/runner/src/main.rs", 'fn main() { let _ = std::fs::read("rom"); println!("ok"); }\n')
        _checked("arch: runner may do IO", not _blocked(runner), runner.stdout)

        unsafe_bad = run("crates/frontend/src/shm.rs", "pub fn f(p: *const u8) -> u8 { unsafe { *p } }\n")
        _checked("arch: block unsafe without SAFETY", _blocked(unsafe_bad), unsafe_bad.stdout)
        unsafe_ok = run(
            "crates/frontend/src/shm2.rs",
            "pub fn f(p: *const u8) -> u8 {\n    // SAFETY: p points into the live shm mapping.\n    unsafe { *p }\n}\n",
        )
        _checked("arch: allow unsafe with SAFETY", not _blocked(unsafe_ok), unsafe_ok.stdout)
        lint_name = run("crates/core/src/lib.rs", "#![deny(unsafe_code)]\npub mod machine;\n")
        no_spdx = run("crates/runner/src/lib.rs", "pub fn f() {}\n", header=False)
        _checked("arch: block a source without the SPDX line", _blocked(no_spdx), no_spdx.stdout)
        asm = run("testroms/phase2/vram_direct/main.s", "@ no header\n", header=False)
        _checked("arch: block a test-ROM source without the SPDX line", _blocked(asm), asm.stdout)
        asm_ok = run("testroms/phase2/vram_direct/ok.s", "@ test ROM\n".replace("@", "@ SPDX-License-Identifier: Apache-2.0\n@", 1), header=False)
        _checked("arch: allow an asm source with the SPDX line", not _blocked(asm_ok), asm_ok.stdout)
        _checked("arch: unsafe_code lint name is not unsafe", not _blocked(lint_name), lint_name.stdout)

        oc = _run(
            "check_architecture.py",
            "opencode",
            _read(str(_write(root, "crates/core/src/oc.rs", SPDX + "use std::rc::Rc;\n"))),
            root=root,
        )
        _checked("arch: opencode exit 1", oc.returncode == 1 and "violation" in oc.stderr.lower(), f"rc={oc.returncode}")


def test_check_architecture_manifests() -> None:
    import check_architecture as arch

    def bad(text: str, ws: dict[str, str] | None = None) -> bool:
        return bool(arch.check_manifest_text(text, "Cargo.toml", ws))

    core_ok = '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dependencies]\nserde = { version = "1", features = ["derive"] }\nthiserror = "2"\n[dev-dependencies]\ninsta = "1"\nproptest = "1"\n[build-dependencies]\nanything = "1"\n'
    _checked("manifest: allow a clean core", not bad(core_ok), str(arch.check_manifest_text(core_ok, "x")))
    cases = (
        ("core -> ratatui", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dependencies]\nratatui = "0.29"\n', None),
        ("core -> rayon (target table)", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[target.\'cfg(unix)\'.dependencies]\nrayon = "1"\n', None),
        ("core -> renamed rand", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dependencies]\nrng = { package = "rand", version = "0.9" }\n', None),
        ("core -> dotted table", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dependencies.crossterm]\nversion = "0.28"\n', None),
        ("core -> workspace rename", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dependencies]\nrng = { workspace = true }\n', {"rng": "rand"}),
        ("core -> runner", '[package]\nname = "ninge-core"\nlicense.workspace = true\n[dev-dependencies]\nninge-runner = { path = "../runner" }\n', None),
        ("runner -> frontend", '[package]\nname = "ninge-runner"\nlicense.workspace = true\n[dependencies]\nninge = { path = "../frontend" }\n', None),
        ("trademark package", '[package]\nname = "ninge-nds"\nlicense.workspace = true\n', None),
        ("non-ninge package", '[package]\nname = "dual-screen-core"\nlicense.workspace = true\n', None),
        ("missing license", '[package]\nname = "ninge-runner"\n', None),
        ("MIT license", '[package]\nname = "ninge-runner"\nlicense = "MIT"\n', None),
        ("workspace license not Apache", '[workspace.package]\nlicense = "MIT OR Apache-2.0"\n', None),
        ("trademark bin", '[package]\nname = "ninge"\nlicense.workspace = true\n[[bin]]\nname = "pokemon-player"\n', None),
    )
    for name, text, ws in cases:
        _checked(f"manifest: block {name}", bad(text, ws), text)
    frontend = '[package]\nname = "ninge"\nlicense.workspace = true\n[[bin]]\nname = "ninge"\npath = "src/main.rs"\n[dependencies]\nninge-core = { path = "../core" }\nratatui = "0.29"\n'
    _checked("manifest: allow the frontend", not bad(frontend), str(arch.check_manifest_text(frontend, "x")))


def test_check_architecture_all() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        _write(root, "crates/core/Cargo.toml", '[package]\nname = "ninge-core"\nlicense.workspace = true\n')
        _write(root, "crates/core/src/lib.rs", SPDX + CORE_OK)
        _write(root, "target/debug/build/x/out.rs", "use std::rc::Rc;\n")  # build output is skipped
        clean = _run("check_architecture.py", "claude", None, root=root, args=("--all",))
        _checked("arch --all: clean tree passes", clean.returncode == 0, clean.stderr)
        _write(root, "crates/core/src/bad.rs", SPDX + "use std::rc::Rc;\n")
        dirty = _run("check_architecture.py", "claude", None, root=root, args=("--all",))
        _checked("arch --all: a violation fails", dirty.returncode == 1 and "bad.rs" in dirty.stderr, dirty.stderr)


# --- check_tests ------------------------------------------------------------------------


TESTS_OK = """\
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_carry_set_when_adc_executes_then_result_includes_carry() {
        let mut cpu = cpu_with_carry();
        cpu.execute(0xE0A1_0002);
        assert_eq!(cpu.reg(1), 3, "if the carry were ignored this would be 2"); // for, while, match
    }

    #[test]
    #[should_panic(expected = "unmapped")]
    fn given_unmapped_page_when_fast_path_reads_then_panics_with_unmapped() {
        read_fast(0x0800_0000);
    }

    #[test]
    fn given_adc_json_suite_when_each_case_runs_then_no_case_differs() {
        let failures = run_json_suite("arm/adc.json");
        assert_eq!(failures, Vec::<CaseDiff>::new());
    }

    proptest! {
        #[test]
        fn given_any_word_when_rotated_32_then_word_is_unchanged(x in any::<u32>()) {
            prop_assert_eq!(x.rotate_right(32), x);
        }
    }
}
"""

TESTS_BAD = (
    ("non-GWT name", "#[test]\nfn adc_works() { assert!(true); }\n"),
    ("vague then", "#[test]\nfn given_a_when_b_then_works() { assert!(true); }\n"),
    ("if in body", "#[test]\nfn given_a_when_b_then_c_is_set() { if x { assert!(true); } }\n"),
    ("for in body", "#[test]\nfn given_a_when_b_then_c_is_set() { for c in cases() { assert_eq!(c, 1); } }\n"),
    ("match in body", "#[test]\nfn given_a_when_b_then_c_is_set() { match x { _ => assert!(true) } }\n"),
    ("let-else in body", "#[test]\nfn given_a_when_b_then_c_is_set() { let Some(v) = x else { return }; assert!(v); }\n"),
    ("for_each in body", "#[test]\nfn given_a_when_b_then_c_is_set() { cases().for_each(|c| assert!(c)); }\n"),
    ("no assert", "#[test]\nfn given_a_when_b_then_c_is_set() { let _ = step(); }\n"),
    ("tokio test", "#[tokio::test]\nasync fn bad_name() { assert!(true); }\n"),
)


def test_check_tests() -> None:
    import check_tests

    found_ok = check_tests.check_source(TESTS_OK, "ok.rs")
    _checked("tests: allow conventional tests", found_ok == [], "\n".join(found_ok))
    for name, text in TESTS_BAD:
        _checked(f"tests: block {name}", check_tests.check_source(text, "bad.rs") != [], text)
    macro = "macro_rules! t { ($n:ident) => { #[test] fn $n() { assert!(true); } } }\nfn helper() { if x {} }\n"
    _checked("tests: skip macro metavariable fns", check_tests.check_source(macro, "m.rs") == [], macro)

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        path = _write(root, "crates/core/src/cpu.rs", TESTS_BAD[0][1])
        hook = _run("check_tests.py", "claude", _read(str(path)), root=root)
        _checked("tests: hook blocks on edit", _blocked(hook), hook.stdout)
        whole = _run("check_tests.py", "claude", None, root=root, args=("--all",))
        _checked("tests --all: a violation fails", whole.returncode == 1 and "cpu.rs" in whole.stderr, whole.stderr)


# --- check_language -----------------------------------------------------------------------

HANGUL = "\uc8fc\uc11d"  # two Hangul syllables, escaped so this file stays English


def test_check_language() -> None:
    import check_language

    latin = "// caf\u00e9, \u03bcs, \u2192, \u25b6, \u2014 are not a foreign script\n"
    _checked("language: allow Latin, Greek, and symbols", check_language.foreign_characters(latin) == [], latin)
    _checked("language: flag Hangul", check_language.foreign_characters(f"// {HANGUL}\n") != [], HANGUL)
    _checked("language: flag kana", check_language.foreign_characters("// \u30c6\u30b9\u30c8\n") != [])
    _checked("language: flag Cyrillic", check_language.foreign_characters("// \u0442\u0435\u0441\u0442\n") != [])

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        comment = _write(root, "crates/core/src/cpu.rs", f"// {HANGUL}\npub fn f() {{}}\n")
        hook = _run("check_language.py", "claude", _read(str(comment)), root=root)
        _checked("language: block a non-English comment", _blocked(hook), hook.stdout)

        _write(root, "README.md", "English.\n")
        translation = _write(root, "README-KO.md", f"{HANGUL}\n")
        ok = _run("check_language.py", "claude", _read(str(translation)), root=root)
        _checked("language: allow an approved translation", not _blocked(ok), ok.stdout)
        orphan = _write(root, "docs/GUIDE-JA.md", "\u30c6\u30b9\u30c8\n")
        lonely = _run("check_language.py", "claude", _read(str(orphan)), root=root)
        _checked("language: block a translation with no original", _blocked(lonely), lonely.stdout)

        whole = _run("check_language.py", "claude", None, root=root, args=("--all",))
        _checked(
            "language --all: violations fail",
            whole.returncode == 1 and "cpu.rs" in whole.stderr and "GUIDE-JA.md" in whole.stderr
            and "README-KO.md" not in whole.stderr,
            whole.stderr,
        )


# --- format_rust / session_context --------------------------------------------------------


def test_format_rust() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        md = _run("format_rust.py", "claude", _read(str(_write(root, "README.md", "x"))), root=root)
        _checked("format: skip non-rust", md.returncode == 0 and md.stdout.strip() == "", md.stdout)
        rs = _run("format_rust.py", "claude", _read(str(_write(root, "a.rs", "fn  main( ){}\n"))), root=root)
        _checked("format: never crashes", rs.returncode == 0 and "Traceback" not in rs.stderr, rs.stderr)


def test_session_context() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        _write(root, "docs/STATUS.md", "# Status\n\n▶ Phase 1 — CPU\n")
        result = _run("session_context.py", "claude", {}, root=root)
        context = json.loads(result.stdout or "{}").get("hookSpecificOutput", {}).get("additionalContext", "")
        _checked("context: names the current phase", "Phase 1 — CPU" in context, result.stdout)
        _write(root, "docs/STATUS.md", "▶ Phase 1\n▶ Phase 2\n")
        two = _run("session_context.py", "pi", {}, root=root)
        _checked("context: two ▶ lines are reported, not guessed", "unknown" in two.stdout, two.stdout)


def test_hooks_survive_old_python() -> None:
    # A hook may resolve to macOS's /usr/bin/python3 (3.9). A crash there is a non-zero exit
    # that Claude/Codex read as "no decision" — fail open. Each hook must still decide.
    candidate = Path("/usr/bin/python3")
    if not candidate.exists():
        return
    probe = subprocess.run([str(candidate), "-c", "import sys; print(sys.version_info >= (3, 10))"], capture_output=True, text=True, check=False)
    if probe.stdout.strip() != "False":
        return
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp).resolve()
        core = _write(root, "crates/core/src/x.rs", "use std::rc::Rc;\n#[test]\nfn bad() {}\n")
        cases = (
            ("guard_cleanroom.py", _read("/tmp/bios7.bin"), _denied),
            ("check_architecture.py", _read(str(core)), _blocked),
            ("check_tests.py", _read(str(core)), _blocked),
            ("check_language.py", _read(str(_write(root, "c.rs", f"// {HANGUL}\n"))), _blocked),
        )
        for script, payload, decided in cases:
            result = _run(script, "claude", payload, root=root, python=str(candidate))
            _checked(f"py3.9: {script} decides", decided(result) and "Traceback" not in result.stderr, result.stderr)
        for script in ("format_rust.py", "session_context.py"):
            result = _run(script, "claude", _read(str(core)), root=root, python=str(candidate))
            _checked(f"py3.9: {script} runs", result.returncode == 0 and "Traceback" not in result.stderr, result.stderr)


# --- check_agent_config (pure helpers) ----------------------------------------------------


def test_config_drift_helpers() -> None:
    if sys.version_info < (3, 11):
        return  # check_agent_config needs tomllib; CI runs this on 3.12
    import check_agent_config as cfg

    canon = "**Forbidden crates in `ninge-core`:** `rand` · `ratatui`.\n"
    _checked(
        "config: a matching list passes",
        cfg.list_mismatch(canon, "**Forbidden crates in `ninge-core`:**", {"rand", "ratatui"}, "x") is None,
    )
    drifted = cfg.list_mismatch(canon, "**Forbidden crates in `ninge-core`:**", {"rand"}, "x")
    _checked("config: a drifted list is reported", drifted is not None and "ratatui" in drifted, str(drifted))
    missing = cfg.list_mismatch("nothing here", "**Forbidden crates in `ninge-core`:**", {"rand"}, "x")
    _checked("config: a missing list line is reported", missing is not None and "exactly once" in missing, str(missing))

    canon_floor = "**Coverage floor:** `90%` line coverage.\n"
    _checked("config: matching coverage floors pass", cfg.coverage_mismatch(canon_floor, "COVERAGE_FLOOR=90\n") is None)
    lowered = cfg.coverage_mismatch(canon_floor, "COVERAGE_FLOOR=80\n")
    _checked("config: a lowered floor is reported", lowered is not None and "80" in lowered, str(lowered))

    digest_a, digest_b = "a" * 64, "b" * 64
    script = f'BLOCKSDS_IMAGE="skylyrac/blocksds@sha256:{digest_a}"\n'
    _checked("config: matching image digests pass", cfg.image_digest_mismatch(f"**BlocksDS image** sha256:{digest_a}", script) is None)
    entry = f"- [x] **BlocksDS image** `sha256:{digest_a}`\n- [x] **Other** `sha256:{digest_b}`\n"
    _checked("config: another sha256 in STATUS is ignored", cfg.image_digest_mismatch(entry, script) is None)
    bumped = cfg.image_digest_mismatch(f"**BlocksDS image** sha256:{digest_b}", script)
    _checked("config: a stale STATUS digest is reported", bumped is not None and "differs" in bumped, str(bumped))
    unrecorded = cfg.image_digest_mismatch("no digest here", script)
    _checked("config: an unrecorded digest is reported", unrecorded is not None and "STATUS" in unrecorded, str(unrecorded))


def main() -> int:
    test_guard_cleanroom()
    test_guard_scan_tracked()
    test_check_architecture_sources()
    test_check_architecture_manifests()
    test_check_architecture_all()
    test_check_tests()
    test_check_language()
    test_format_rust()
    test_session_context()
    test_hooks_survive_old_python()
    test_config_drift_helpers()

    if FAILURES:
        print("hook selftest FAILED:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    print("hook selftest: all checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
