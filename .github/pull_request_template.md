<!-- Title: `<PREFIX>: <behavior>` — see .claude/skills/commit/SKILL.md. One PR = one hardware feature or one bug. English only. -->

## Hardware

<!-- The feature or bug, and the docs/hw/ note it follows. -->

## Tests

<!-- Which tests pin it. Confirm the new ones failed before the change. -->

## Golden frames

<!-- Every hash added or changed, and why the new frame is right. "None" if none. -->

## Unverified

<!-- Any TODO(hw-unverified) added, and how it could be verified. "None" if none. -->

## Decision · Phase

<!-- The `decision` issue this acts on (Decided in #N), if any. The phase this belongs to. -->

## Checklist

- [ ] `./scripts/verify.sh` passes
- [ ] No ROM, save, dump, extracted data, or GPL-derived code (`cleanroom-auditor`)
- [ ] Reviewed by a session other than the one that wrote the code
