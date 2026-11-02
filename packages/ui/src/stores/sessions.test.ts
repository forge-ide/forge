import { expect, it } from "vitest";
import { fakeDaemon, settle } from "../testing/daemon";
import { createSessionsStore } from "./sessions";
it("loads the project sessions and marks them stale across disconnects", async () => {
  const d = fakeDaemon(() => [{ id: "ses_test", title: "Example" }]);
  const s = createSessionsStore(d.conn, "prj_test");
  await settle();
  expect(s.state.sessions[0]?.title).toBe("Example");
  expect(d.calls[0]?.params).toEqual({ projectId: "prj_test" });
  d.setState({ kind: "reconnecting" });
  expect(s.state.stale).toBe(true);
  s.dispose();
  expect(d.listenerCount()).toBe(0);
});
