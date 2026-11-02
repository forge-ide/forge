import { spawn } from "node:child_process";
import { open } from "node:fs/promises";
export async function startProcess(
  command,
  args,
  { env = process.env, logPath, cwd = process.cwd() } = {},
) {
  const log = await open(logPath, "w", 0o600);
  let exited = false,
    error;
  const child = spawn(command, args, {
    env,
    cwd,
    detached: true,
    stdio: ["ignore", log.fd, log.fd],
  });
  const ended = new Promise((resolve) => {
    child.once("exit", (code, signal) => {
      exited = true;
      resolve({ code, signal });
    });
    child.once("error", (e) => {
      error = e;
      exited = true;
      resolve({ error: e.message });
    });
  });
  const stop = (signal) => {
    try {
      process.kill(-child.pid, signal);
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  };
  const groupAlive = () => {
    if (typeof child.pid !== "number") return false;
    try {
      process.kill(-child.pid, 0);
      return true;
    } catch (error) {
      if (error.code === "ESRCH") return false;
      throw error;
    }
  };
  async function waitGroup(milliseconds) {
    const deadline = Date.now() + milliseconds;
    while (groupAlive() && Date.now() < deadline)
      await new Promise((resolve) => setTimeout(resolve, 20));
  }
  let closing;
  return {
    child,
    assertRunning() {
      if (exited)
        throw new Error(
          `${command} exited early${error ? `: ${error.message}` : ""}; see ${logPath}`,
        );
    },
    close() {
      return (closing ??= (async () => {
        try {
          if (groupAlive()) stop("SIGTERM");
          await waitGroup(3000);
          if (groupAlive()) {
            stop("SIGKILL");
            await waitGroup(1000);
          }
          await ended;
          if (groupAlive()) throw new Error(`Owned process group did not exit: ${command}`);
        } finally {
          await log.close();
        }
      })());
    },
  };
}
