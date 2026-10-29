/**
 * Pi extension — an adapter for the ninge agent gates.
 *
 * The rules and logic live in scripts/agent_hooks/ and are shared with Claude Code, Codex,
 * Cursor, and OpenCode (AGENTS.md, .agents/README.md: "one body, five adapters"). This
 * file only translates Pi tool calls into the hook stdin shape and maps the exit code.
 *
 * Pi loads project-local extensions from `.pi/extensions/*.ts` only AFTER the project is
 * trusted. Approve trust once (interactive prompt, `/trust`, or `pi --approve`), otherwise
 * this extension never runs.
 *
 * Pi has no subagents; the roles in .claude/agents/ do not apply. Use the skills under
 * `.agents/skills/` and the AGENTS.md canon instead.
 */

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const PYTHON = "python3";
const HOOK_TIMEOUT_MS = 20_000;
const HOOK_SCRIPT = join("scripts", "agent_hooks", "guard_cleanroom.py");
const POST_HOOKS = ["check_architecture.py", "check_tests.py", "check_language.py"];

/** Walk up from `start` until we find the checkout that holds the hook scripts. */
function findRepoRoot(start: string): string | null {
	let dir = start;
	for (;;) {
		if (existsSync(join(dir, HOOK_SCRIPT))) return dir;
		const parent = dirname(dir);
		if (parent === dir) return null;
		dir = parent;
	}
}

function runHook(
	root: string,
	name: string,
	payload: unknown,
	cwd: string,
): { code: number; stderr: string } {
	const script = join(root, "scripts", "agent_hooks", name);
	const res = spawnSync(PYTHON, [script, "--tool", "pi"], {
		input: JSON.stringify(payload),
		encoding: "utf8",
		timeout: HOOK_TIMEOUT_MS,
		// Resolve the hook's relative paths against the session cwd, not the agent process.
		cwd,
	});
	// status is null on spawn failure or timeout — treat that as a non-zero (fail closed).
	return { code: res.status ?? 1, stderr: (res.stderr || "").trim() };
}

const pathOf = (input: Record<string, unknown> | undefined): string => {
	const candidate = input?.filePath ?? input?.file_path ?? input?.path ?? input?.target_file;
	return typeof candidate === "string" ? candidate : "";
};

export default function (pi: ExtensionAPI) {
	// Pre — block clean-room violations. ANY non-zero exit blocks (fail closed).
	pi.on("tool_call", async (event, ctx) => {
		try {
			const root = findRepoRoot(ctx.cwd);
			if (root === null) {
				return { block: true, reason: "ninge guard: hook scripts not found above cwd" };
			}

			const name = event.toolName;
			const input = event.input as Record<string, unknown> | undefined;
			let payload: unknown = null;

			if (name === "bash" || name === "powershell") {
				payload = { tool_input: { command: input?.command ?? "" } };
			} else if (["read", "write", "edit", "grep", "find", "ls"].includes(name)) {
				const path = pathOf(input);
				if (!path) return;
				payload = { file_path: path, content: input?.content, new_string: input?.newText };
			}
			if (payload === null) return;

			const { code, stderr } = runHook(root, "guard_cleanroom.py", payload, ctx.cwd);
			if (code !== 0) {
				return { block: true, reason: stderr || `blocked by guard_cleanroom (exit ${code})` };
			}
		} catch (error) {
			return { block: true, reason: `ninge guard error: ${String(error)}` };
		}
	});

	// Post — rustfmt the edited Rust, then surface architecture / test / language violations.
	pi.on("tool_result", async (event, ctx) => {
		try {
			if (event.toolName !== "write" && event.toolName !== "edit") return;
			const path = pathOf(event.input as Record<string, unknown> | undefined);
			if (!path) return;
			const root = findRepoRoot(ctx.cwd);
			if (root === null) return;

			const stdin = { file_path: path };
			if (path.endsWith(".rs")) {
				const fmt = runHook(root, "format_rust.py", stdin, ctx.cwd);
				if (fmt.stderr) ctx.ui.notify?.(fmt.stderr, "info");
			}

			const problems = POST_HOOKS.map((name) => runHook(root, name, stdin, ctx.cwd))
				.filter((res) => res.code !== 0)
				.map((res) => res.stderr);
			if (problems.length) {
				ctx.ui.notify?.("ninge gate violation (AGENTS.md)", "error");
				return {
					content: [...(event.content ?? []), { type: "text" as const, text: problems.join("\n\n") }],
				};
			}
		} catch {
			// Never break the tool on a post-hook error.
		}
	});
}
