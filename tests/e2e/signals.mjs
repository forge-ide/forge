/** Termination must finish owned-resource cleanup before Node bypasses async finally. */
export function onTermination(cleanup, { abort = () => {} } = {}) {
  let stopping = false;
  const handler = async (signal) => {
    if (stopping) return;
    stopping = true;
    abort();
    const code = signal === "SIGINT" ? 130 : 143;
    try {
      await cleanup();
    } catch (error) {
      console.error(`Cancellation cleanup failed: ${error.message}`);
    } finally {
      process.exit(code);
    }
  };
  const interrupt = () => void handler("SIGINT"),
    terminate = () => void handler("SIGTERM");
  process.on("SIGINT", interrupt);
  process.on("SIGTERM", terminate);
  return () => {
    process.off("SIGINT", interrupt);
    process.off("SIGTERM", terminate);
  };
}
