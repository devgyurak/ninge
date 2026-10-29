# Agent configuration map

This repository supports five coding agents. **The canon is the single root
`AGENTS.md`.** Every tool converges there from its own entrypoint. To change a rule, edit
`AGENTS.md` — the files below are per-tool wiring.

| Tool | Reads | Note |
| --- | --- | --- |
| **Codex** | `AGENTS.md` + `.agents/skills/`<sup>※</sup> + `.codex/` | Subagents in `.codex/agents/*.toml`, hooks in `.codex/hooks.json`<sup>※</sup> |
| **Claude Code** | `CLAUDE.md` → `@AGENTS.md` + `.claude/skills/` + `.claude/settings.json` | Subagents in `.claude/agents/` |
| **Pi** | `AGENTS.md` + `.agents/skills/` + `.pi/settings.json` | Gates via `.pi/extensions/` (project trust required); **no subagents** |
| **Cursor** | `AGENTS.md` + `.cursor/rules/*.mdc` + `.cursor/hooks.json`<sup>※</sup> | `.mdc` files are three scope-focused highlights |
| **OpenCode** | `AGENTS.md` + `.opencode/agents/` + `.opencode/opencode.json` | Gates run through `.opencode/plugin/ninge-gates.js` |

<sup>※</sup> **Unverified.** `SKILL.md` discovery on Codex, Cursor hooks, and Codex hooks follow
each tool's own docs, but have not been observed running in this repository. Since the canon
lives in `AGENTS.md` — which all five read — the rules reach an agent even if a given tool
cannot open a skill or run a hook, and `./scripts/verify.sh` + CI hold the line regardless.

Skills are **not** duplicated per tool. The one thing that is duplicated is the
subagents, because `.opencode/agents/` and `.codex/agents/` use different frontmatter
schemas.

## File layout

```
AGENTS.md                       ← canon (read directly by Codex · Pi · Cursor · OpenCode)
CLAUDE.md                       ← imports @AGENTS.md + Claude-specific notes
docs/PLAN.md                    ← the project plan, cited as "plan §N"
docs/PLAN-KO.md                 ← its approved Korean translation
docs/STATUS.md                  ← current phase (the one ▶ line) + open decisions
docs/hw/README.md               ← hardware-note template (the clean-room boundary)

.claude/
├── agents/                     ← Claude Code subagents
│   ├── architecture-guard.md
│   ├── cleanroom-auditor.md
│   └── test-author.md
├── settings.json               ← Claude hooks + permissions
└── skills/                     ← skill originals (shared by all five tools, SKILL.md standard)
    ├── verify/SKILL.md
    ├── add-hw-feature/SKILL.md
    ├── trace-bug/SKILL.md
    └── commit/SKILL.md

.agents/
├── README.md                   ← this file
└── skills → ../.claude/skills  ← symlink (Codex · Pi read .agents/skills)

.codex/
├── config.toml
├── hooks.json
└── agents/                     ← same subagents, Codex schema
    ├── architecture_guard.toml
    ├── cleanroom_auditor.toml
    └── test_author.toml

.opencode/
├── opencode.json
├── plugin/ninge-gates.js       ← hook adapter → scripts/agent_hooks/
└── agents/                     ← same subagents, OpenCode schema
    ├── architecture-guard.md
    ├── cleanroom-auditor.md
    └── test-author.md

.cursor/
├── hooks.json
└── rules/
    ├── architecture.mdc        ← alwaysApply: true
    ├── rust-style.mdc          ← globs: crates/**/*.rs
    └── testing.mdc             ← globs: tests, testroms

.pi/
├── README.md
├── settings.json               ← project settings (trust required to load)
└── extensions/
    └── ninge-gates.ts          ← gate adapter → scripts/agent_hooks/ (trust required)

scripts/
├── verify.sh                   ← the one gate (local and CI)
├── check_agent_config.py       ← integrity check (CI gate)
└── agent_hooks/                ← hook bodies (shared by Claude · Codex · Cursor · OpenCode · Pi)
    ├── _hookio.py              ← per-tool stdin/stdout protocol
    ├── _rustscan.py            ← tiny Rust lexer (comments/literals, test fn bodies)
    ├── guard_cleanroom.py
    ├── format_rust.py
    ├── check_architecture.py
    ├── check_tests.py
    ├── check_language.py
    ├── session_context.py
    └── selftest.py
```

## Subagents

All three exist in `.claude/agents/`, `.opencode/agents/`, and `.codex/agents/`. The body
is the same; only the frontmatter schema differs.

- **architecture-guard** (read-only) — crate graph, core purity, single ownership,
  plain-data state, hot-path dispatch and allocation, scheduler determinism, phase scope.
  After `crates/` or a `Cargo.toml`.
- **cleanroom-auditor** (read-only) — no dumps or extracted data, no GPL-derived code, every
  behavior traced to `docs/hw/`, guesses marked, trademarks, monetization, dependency
  licenses. After `hle/` · `spi/` · `cart/` · `testroms/` · fixtures · `docs/hw/` · README ·
  any binary file.
- **test-author** (writable) — writes tests to spec: GWT names, no control flow, harness
  functions, determinism, golden frames. Before an implementation (the failing test), or when
  a change lacks coverage.

## Skills

- **verify** — run `./scripts/verify.sh` and diagnose failures
- **add-hw-feature** — plan §7.1: phase check → `docs/hw/` note → failing test → implement →
  debugger exposure → verify → `decision` issue
- **trace-bug** — plan §7.2: replay → trace diff → spec → failing test → fix
- **commit** — commit/PR/branch conventions and prefix boundaries

## Hooks

The bodies live once in `scripts/agent_hooks/`. Each tool's hook config calls the same
script with a `--tool` flag, so behavior changes in one place.

| Hook | Where | Does |
| --- | --- | --- |
| `session_context.py` | SessionStart (Claude) | Prints the current phase and the non-negotiables |
| `guard_cleanroom.py` | Pre | Blocks dumps, ROM/save bytes, GPL emulator source, GPL notices in code. `--scan-tracked` checks `git ls-files` |
| `format_rust.py` | Post | `rustfmt --edition 2024` on edited `*.rs`; a no-op until rustfmt is installed |
| `check_architecture.py` | Post | Core purity, `unsafe` without `SAFETY:`, Apache-2.0 SPDX line and package license, crate graph, crate names. Cursor runs it on `stop`. `--all` scans the repo |
| `check_tests.py` | Post | GWT names, no control flow, asserts present. Cursor runs it on `stop`. `--all` scans the repo |
| `check_language.py` | Post | English-only text outside `-{code}` translations; a translation needs its English original. `--all` scans the repo |

Enforcement is **best-effort**: it only fires for a file an agent edits through a wired tool.
A change made by `Bash`/`sed`, or in a harness whose hook is not loaded, skips it — so every
rule that matters also runs in `./scripts/verify.sh` and CI (AGENTS.md §7). The clean-room
guard fails **closed** (claude/codex/cursor emit a deny; opencode/pi exit non-zero; Cursor sets
`failClosed: true`).

## Maintenance

- **To change a rule, edit `AGENTS.md`.** All five tools follow.
- **Change a subagent in all three copies** (`.claude/` · `.opencode/` · `.codex/`). The
  body must match. `scripts/check_agent_config.py` verifies this — Codex
  `developer_instructions` must equal the Claude body verbatim.
- **A list the hooks enforce is stated once in AGENTS.md** — the core-forbidden crates (§2)
  and the GPL emulators (§3.3). Change the hook's set and the AGENTS.md line together; the
  config check fails if they differ.
- **Subagents do not exist on Pi.** The three roles apply to Claude Code, OpenCode, and
  Codex; on Pi, read the role file as a fresh session's opening prompt instead.
- **Edit a skill only in `.claude/skills/`** — Codex·Pi see the same file via the
  `.agents/skills` symlink. On a checkout that cannot follow the symlink, skill discovery
  is empty there but `AGENTS.md` still works.
- **Do not replicate `AGENTS.md` into `.cursor/rules/`.** Cursor reads `AGENTS.md`;
  `.mdc` files are **scope-focused highlights** only.
- **Rule vs. skill**: always-true rules (crate graph, clean-room invariants) belong in
  `AGENTS.md`; a procedure needed only for a specific task (adding a hardware feature,
  committing) belongs in a skill. `AGENTS.md` is loaded into every session; skills load on
  demand.
- **After editing agent config, run** `python3 scripts/check_agent_config.py`.
  CI (`.github/workflows/agents.yml`) runs the same check plus the hook self-tests.
