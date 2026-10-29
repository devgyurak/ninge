/**
 * OpenCode plugin — an adapter for the ninge agent gates.
 *
 * The rules and the decision logic do not live here. They live in scripts/agent_hooks/
 * and are shared with Claude Code, Codex, Cursor, and Pi (AGENTS.md, .agents/README.md:
 * "one body, five adapters"). This file only translates OpenCode tool arguments, runs the
 * shared scripts through Bun Shell ($), and maps the exit code.
 *
 * Bun Shell has no `.stdin()` method — the payload is piped in as a Response body.
 * A non-zero exit means the Python hook denied/blocked, so we throw.
 *
 * NOTE on patch tools: OpenCode's `patch`/`apply_patch` deliver the edit as `patchText`,
 * not as a path. We forward it as `command` so the hook's patch parser (`*** … File: …`)
 * reads the touched paths. Without this the patch tool would be skipped entirely.
 */

const hookPath = (root, name) => `${root}/scripts/agent_hooks/${name}`;

/** First path-like argument among the many spellings the tools use. */
const pathOf = (args) =>
  args?.filePath ?? args?.file_path ?? args?.path ?? args?.target_file ?? "";

/** Patch text from a patch/apply_patch tool, whichever key it arrived under. */
const patchOf = (args) => args?.patchText ?? args?.patch_text ?? args?.patch ?? "";

/** The stdin payload for either a path-based tool or a patch-based one. */
const payloadFor = (args) => {
  const patch = patchOf(args);
  if (patch) return JSON.stringify({ tool_input: { command: patch } });
  return JSON.stringify({
    file_path: pathOf(args),
    content: args?.content,
    new_string: args?.newString ?? args?.new_string,
    edits: args?.edits,
  });
};

const PATH_TOOLS = new Set(["read", "write", "edit", "grep", "glob", "list"]);
const PATCH_TOOLS = new Set(["patch", "apply_patch"]);
const EDIT_TOOLS = new Set(["write", "edit", "patch", "apply_patch"]);
const FETCH_TOOLS = new Set(["webfetch", "fetch"]);
const POST_HOOKS = ["check_architecture.py", "check_tests.py", "check_language.py"];

export const NingeGates = async ({ $, directory, worktree }) => {
  // `worktree` is the repo root even when OpenCode was started in a subdirectory; fall back
  // to `directory` if the host does not provide it.
  const root = worktree ?? directory;

  const runHook = async (name, stdin) => {
    // Script path is resolved from the repo root, but the process cwd must be the session
    // directory: the model's relative paths are relative to the session, not the repo root.
    const res = await $`${hookPath(root, name)} --tool opencode < ${new Response(stdin)}`
      .cwd(directory)
      .quiet()
      .nothrow();
    return { exitCode: res.exitCode, stderr: res.stderr?.toString() ?? "" };
  };

  /** Pre — block clean-room violations before they happen. Fail closed on any non-zero. */
  const guard = async (stdin) => {
    const res = await runHook("guard_cleanroom.py", stdin);
    if (res.exitCode !== 0) {
      throw new Error(res.stderr.trim() || `blocked by guard_cleanroom (exit ${res.exitCode})`);
    }
  };

  return {
    "tool.execute.before": async (input, output) => {
      const args = output?.args ?? {};
      if (input.tool === "bash" || input.tool === "shell") {
        await guard(JSON.stringify({ tool_input: { command: args.command ?? "" } }));
        return;
      }
      if (FETCH_TOOLS.has(input.tool)) {
        await guard(JSON.stringify({ tool_input: { url: args.url ?? "" } }));
        return;
      }
      if (PATCH_TOOLS.has(input.tool)) {
        await guard(payloadFor(args));
        return;
      }
      if (!PATH_TOOLS.has(input.tool)) return;
      if (!pathOf(args)) return;
      await guard(payloadFor(args));
    },

    /** Post — rustfmt the edited Rust, then report architecture / test / language violations. */
    "tool.execute.after": async (input, output) => {
      if (!EDIT_TOOLS.has(input.tool)) return;
      const args = output?.args ?? {};
      const payload = payloadFor(args);

      if (patchOf(args) || pathOf(args).endsWith(".rs")) {
        const fmt = await runHook("format_rust.py", payload);
        if (fmt.stderr.trim()) console.warn(fmt.stderr.trim());
      }

      const problems = [];
      for (const name of POST_HOOKS) {
        const res = await runHook(name, payload);
        if (res.exitCode !== 0) problems.push(res.stderr.trim() || `${name} reported a violation`);
      }
      if (problems.length) throw new Error(problems.join("\n\n"));
    },
  };
};
