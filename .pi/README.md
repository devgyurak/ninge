# Pi usage

ninge supports Pi through the canon, the skills, and one gate extension.

## Files

- `.pi/settings.json` — project settings (Pi reads this, not a `config.toml`).
- `.pi/extensions/ninge-gates.ts` — the gate adapter. It shells out to the shared
  `scripts/agent_hooks/` bodies, so behavior changes in one place.
- `AGENTS.md` — the canon. Pi reads it.
- `.agents/skills/` — the skills (a symlink to `.claude/skills/`, the single original).

## Project trust is required

Pi loads project-local `.pi/` resources and extensions **only after the project is
trusted**. On the first run Pi asks; approve it (or use `/trust`, or `pi --approve` for a
single run). Until then the extension does not run and the gates are inactive — rely on
`./scripts/verify.sh` and CI.

## Session start

Pi does not run the SessionStart hook. Start each session by reading the `▶` line in
`docs/STATUS.md` — that is the current phase, and work stays inside it.

## Subagents

**Pi has no subagents.** The roles in `.claude/agents/` (architecture-guard,
cleanroom-auditor, test-author) do not apply. To keep the same separation, run a fresh
session and give it one role from `.claude/agents/<name>.md` as the opening prompt — never
review in the session that wrote the code.

## Minimal setup

```bash
./scripts/verify.sh
```

## Skills

Invoke a skill or read `.agents/skills/<name>/SKILL.md` directly:

| Skill | When |
| --- | --- |
| `verify` | run the full gate and diagnose failures |
| `add-hw-feature` | implement a hardware feature (plan §7.1) |
| `trace-bug` | investigate an emulation bug (plan §7.2) |
| `commit` | commit / PR / branch conventions |
