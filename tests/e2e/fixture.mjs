import { mkdtemp, mkdir, writeFile, readFile, realpath, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { connectRpc, until } from "./rpc.mjs";
import { startProcess } from "./process.mjs";
export async function startFixture({ artifactDir, signal }) {
  await mkdir(artifactDir, { recursive: true });
  const root = await realpath(await mkdtemp("/tmp/forge-e2e-"));
  const env = {
    ...process.env,
    HOME: root,
    FORGE_DATA_DIR: join(root, "data"),
    FORGE_RUNTIME_DIR: join(root, "run"),
    FORGE_CONFIG_DIR: join(root, "config"),
    FORGE_MANAGED_POLICY: join(root, "managed.toml"),
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Forge test",
    GIT_AUTHOR_EMAIL: "test@localhost",
    GIT_COMMITTER_NAME: "Forge test",
    GIT_COMMITTER_EMAIL: "test@localhost",
    RUST_LOG: "forged=info",
  };
  let rpc,
    daemon,
    closed = false;
  const close = async () => {
    if (closed) return;
    closed = true;
    try {
      await rpc?.close();
    } finally {
      try {
        await daemon?.close();
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    }
  };
  try {
    for (const name of ["data", "run", "config"]) await mkdir(join(root, name), { mode: 0o700 });
    const repo = join(root, "repo");
    await mkdir(repo);
    const git = (...args) =>
      execFileSync("git", ["-C", repo, ...args], { env, stdio: "pipe" }).toString();
    git("init", "--quiet", "--initial-branch=main");
    await writeFile(join(repo, "README.md"), "# Review fixture\n");
    git("add", "README.md");
    git("commit", "--quiet", "-m", "Initial fixture");
    await writeFile(join(repo, ".git/info/exclude"), ".forge/agents/\n.worktrees/\n");
    const bins = resolve("target/debug");
    await mkdir(join(repo, ".forge/agents"), { recursive: true });
    signal?.throwIfAborted();
    daemon = await startProcess(join(bins, "forged"), [], {
      env,
      logPath: join(artifactDir, "forged.log"),
    });
    rpc = await until(
      async () => {
        daemon.assertRunning();
        return await connectRpc(join(root, "run/forged.sock"));
      },
      () => true,
      { label: "private daemon socket", signal },
    );
    const generated = await readFile("packages/client/src/generated/api.ts", "utf8");
    const apiVersion = /export const API_VERSION = "([^"]+)"/.exec(generated)?.[1];
    if (!apiVersion) throw new Error("Missing generated API version");
    await rpc.call("server.hello", { client: "forge-e2e", apiVersion });
    const project = await rpc.call("project.open", { path: repo });
    git("add", "forge.toml");
    git("commit", "--quiet", "-m", "Configure fixture");
    async function createSession(title, script) {
      const id = title.toLowerCase().replaceAll(" ", "-");
      const scriptPath = join(repo, ".forge/agents", `${id}.yaml`);
      await writeFile(scriptPath, script);
      await writeFile(
        join(repo, ".forge/agents", `${id}.toml`),
        `agent = "fake"\ncommand = [${JSON.stringify(join(bins, "forge-fake-agent"))}, ${JSON.stringify(scriptPath)}]\nsandbox = "unsandboxed"\nallow_unsandboxed = true\n`,
      );
      const session = await rpc.call("session.create", {
        projectId: project.id,
        agentProfile: id,
        title,
        prompt: "Add retry behavior",
        mode: "accept-edits",
        sandbox: "unsandboxed",
        branch: `feat/${id}`,
      });
      await until(
        () => rpc.call("session.get", { sessionId: session.id }),
        (s) => s.state === "review",
        { label: `${title} checkpoint`, signal },
      );
      return session;
    }
    const loop = await createSession(
      "Review loop",
      await readFile("tests/fixtures/agents/review-loop.yaml", "utf8"),
    );
    function lines(file, count) {
      return Array.from({ length: count }, (_, i) =>
        i === count - 1
          ? `// FINAL ${file} ${count}\n`
          : `// ${file} line ${String(i + 1).padStart(4, "0")}\n`,
      ).join("");
    }
    async function perf(title, count, size) {
      const writes = Array.from(
        { length: count },
        (_, i) => `  - write: { path: "file-${i}.rs", content: ${JSON.stringify(lines(i, size))} }`,
      ).join("\n");
      const session = await createSession(title, `steps:\n${writes}\n  - end_turn\n`);
      const changeset = await rpc.call("review.changeset", { sessionId: session.id });
      if (changeset.files.reduce((n, f) => n + f.additions + f.deletions, 0) !== 5000)
        throw new Error("Performance fixture must change exactly 5,000 lines");
      return session;
    }
    const opening = await perf("Review performance", 5, 1000),
      large = await perf("Review large file", 1, 5000);
    return { env, root, repo, project, loop, opening, large, rpc, close };
  } catch (error) {
    await close();
    throw error;
  }
}
