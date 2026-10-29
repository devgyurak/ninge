---
name: commit
description: Commit-message, PR-title, PR-body, and branch-name conventions for ninge. Use right before committing or opening a PR, or when the user asks to commit, open a PR, word a message, or choose a prefix. Covers choosing among ADD/FIX/REF/DEL/DOCS/UPT, the one-feature-or-one-bug rule, golden-hash reasons, and release-note labels.
---

# Commit · PR · branch names

> Commit only when the user asks. This skill is "how", not "when".

The commit subject, PR title, and branch share **one prefix**. One PR carries one kind of
change — **one hardware feature or one bug** (plan §7.3) — so there is exactly one prefix.

```
<PREFIX>: <summary>

<body — the "why">
```

Subjects, bodies, PR text, and issue text are **English**, whatever language the
conversation is in (`AGENTS.md` §9).

## 1. Look at what changed first

Decide the prefix from the diff, not from memory.

```bash
git status --short
git diff --stat
git diff            # unstaged
git diff --staged   # staged
```

## 2. Pick one prefix

| Prefix | When | Example |
| --- | --- | --- |
| `ADD:` | Something new — hardware behavior · feature · test · workflow · dependency | `ADD: IPC FIFO with error flags` |
| `FIX:` | A bug — behavior differed from the hardware | `FIX: keep the LDM base when it is in the list` |
| `REF:` | Structure only — **observable behavior unchanged** | `REF: split the bus views into modules` |
| `DEL:` | Removal — feature · file · dead code | `DEL: unused Thumb decode path` |
| `DOCS:` | Documentation only | `DOCS: note the DIVCNT busy-bit timing` |
| `UPT:` | A dependency **version bump** | `UPT: ratatui 0.30` |

Uppercase, colon, one space. Not `Add:` or `ADD :`.

### When the boundary is unclear

**1. Did observable behavior change?** Would any test, golden frame, or savestate differ?
- No → `REF:`.
- New behavior → `ADD:`.
- Wrong behavior corrected → `FIX:` (with the test that failed before).

**2. A dependency is added vs. updated.**
- New crate in a `Cargo.toml` → `ADD:`.
- Only a version was raised, or `Cargo.lock` only → `UPT:`.

**3. Docs and code together is not `DOCS:`.** Code plus its `docs/hw/` note takes the code
prefix.

**3-1. Docs are `DOCS:` even when added.** A new `docs/hw/` note alone is `DOCS:`. Agent
settings are documentation too — everything under `.claude/` · `.agents/` · `.codex/` ·
`.cursor/` · `.opencode/` · `.pi/`.

**But `scripts/` is code** — it runs. A hook or gate change is `ADD:`/`FIX:`.
`.github/workflows/` is likewise code.

**4. Tests only → `ADD:`.** Structure-only cleanup of existing tests → `REF:`.

**5. Two prefixes fit → split the commit.**

## 3. Subject line

- English, imperative or noun phrase, no trailing period.
- Around 50 characters including the prefix.
- State the **behavior**, not a list of files, and not a game's name — name the hardware.

```
Good:  FIX: keep the LDM base when it is in the list
Bad:   FIX: cpu.rs
Bad:   FIX: fix Mario Kart freeze
```

## 4. Body — **why**

**What** is already in the diff. Emulator code is full of deliberate oddities; if the reason
is not recorded, the next person "tidies" it into a regression.

```
FIX: keep the LDM base when it is in the list

On ARMv4 an LDM with writeback whose base is also in the register list must
not let the writeback clobber the loaded value (docs/hw/arm-block-transfer.md).
We wrote back after the loads, so the base ended up as the incremented
address. The JSON suite case ldm_wb_base_in_list now passes.
```

The body is **mandatory** when the change touches HLE, synthetic firmware, cart, or
anything in AGENTS.md §3, when it uses `unsafe` (cite the measurement), or when it changes a
golden hash.

## 5. Branch name

`<prefix lowercase>/<summary-kebab>`, cut from `develop`; the PR targets `develop` (`main` is
release-only).

```
add/ipc-fifo
fix/ldm-base-in-list
docs/divcnt-timing
```

## 6. PR body

Use `.github/pull_request_template.md`. It must say:

- **Hardware** — the feature or bug, and its `docs/hw/` note.
- **Tests** — which tests pin it, and that the new ones failed first.
- **Golden frames** — every hash added or changed, and **why the new frame is right**
  (plan §7.3). "Updated hashes" alone is not a reason.
- **Unverified** — any `TODO(hw-unverified)` added.
- **Decision** — the `decision` issue it acts on, if any (`Decided in #N`).
- **Phase** — the phase it belongs to.

## 7. Labels

| Prefix | Label | Release note |
| --- | --- | --- |
| `ADD:` | `feature` — but `test` if tests only, `ci` if workflows/gates only, `dependencies` if a new dependency | ✨ (or 🧹 / 📦 for the exceptions) |
| `FIX:` | `bug` | 🐛 Bug fix |
| `REF:` | `refactor` (`ci` / `test` if only CI / tests) | 🧹 Internal |
| `DEL:` | `refactor` | 🧹 Internal |
| `DOCS:` | `documentation` | 📝 Documentation |
| `UPT:` | `dependencies` | 📦 Dependencies |

## 8. Pre-commit checklist

- [ ] Does the prefix match the diff? (behavior changed but marked `REF:` is wrong)
- [ ] One hardware feature or one bug per PR?
- [ ] Does the subject state the **behavior**?
- [ ] Is the **why** recorded for anything surprising?
- [ ] Did `./scripts/verify.sh` pass? (`verify` skill)
- [ ] Every golden hash change has its reason in the PR?
- [ ] No ROM, save, dump, extracted data, or GPL text slipped in? (`cleanroom-auditor`)
- [ ] All English?
