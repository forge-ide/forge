// Async store methods deliberately read the current snapshot at invocation time.
/* eslint-disable solid/reactivity */
import { createStore } from "solid-js/store";
import {
  DaemonError,
  type Changeset,
  type FileChange,
  type Hunk,
  type Session,
  type ShipStrategy,
  type Thread,
} from "@forge/client";
import type { DaemonConnection } from "../client/connection";
export function errorText(error: unknown): string {
  if (error instanceof DaemonError)
    return `${error.message}${error.data ? "\n" + JSON.stringify(error.data, null, 2) : ""}`;
  return error instanceof Error ? error.message : String(error);
}
export function fileKey(file: Pick<FileChange, "repo" | "path">): string {
  return JSON.stringify([file.repo ?? "", file.path]);
}
export function collapsedFile(file: FileChange): boolean {
  return (
    file.binary ||
    file.additions + file.deletions > 1500 ||
    /(?:^|\/)(?:[^/]*\.lock|package-lock\.json|pnpm-lock\.yaml|yarn\.lock|[^/]*\.generated\.[^/]+|generated)(?:\/|$)/i.test(
      file.path,
    )
  );
}
export type ReviewState = {
  status: "loading" | "ready" | "error";
  stale: boolean;
  changeset: Changeset | undefined;
  fullChangeset: Changeset | undefined;
  session: Session | undefined;
  threads: Thread[];
  threadsStatus: "loading" | "ready" | "error";
  threadsError: string | undefined;
  selected: FileChange | undefined;
  fromCheckpoint: string | undefined;
  sinceLast: boolean;
  hunks: Hunk[];
  diffStatus: "empty" | "collapsed" | "loading" | "ready" | "error" | "withheld";
  diffError: string | undefined;
  error: string | undefined;
  actionError: string | undefined;
  busy: boolean;
  withheld: string[];
};
export function createReviewStore(conn: DaemonConnection, sessionId: string) {
  const [state, set] = createStore<ReviewState>({
    status: "loading",
    stale: conn.state.kind !== "connected",
    changeset: undefined,
    fullChangeset: undefined,
    session: undefined,
    threads: [],
    threadsStatus: "loading",
    threadsError: undefined,
    selected: undefined,
    fromCheckpoint: undefined,
    sinceLast: false,
    hunks: [],
    diffStatus: "empty",
    diffError: undefined,
    error: undefined,
    actionError: undefined,
    busy: false,
    withheld: [],
  });
  let disposed = false,
    revision = 0,
    diffRevision = 0,
    threadRevision = 0,
    sessionRevision = 0;
  let timer: ReturnType<typeof setTimeout> | undefined,
    refreshAll = false;
  const pathParams = (file: FileChange) => ({
    sessionId,
    path: file.path,
    ...(file.repo === undefined ? {} : { repo: file.repo }),
  });
  const scope = () =>
    state.fromCheckpoint === undefined ? {} : { fromCheckpoint: state.fromCheckpoint };
  async function refreshThreads() {
    const rev = ++threadRevision;
    try {
      const threads = await conn.call("review.threads", { sessionId });
      if (!disposed && rev === threadRevision)
        set({ threads, threadsStatus: "ready", threadsError: undefined });
    } catch (e) {
      if (!disposed && rev === threadRevision)
        set({ threadsStatus: "error", threadsError: errorText(e) });
    }
  }
  async function loadDiff() {
    const file = state.selected;
    if (!file) return;
    const rev = ++diffRevision;
    set({ diffStatus: "loading", hunks: [], diffError: undefined });
    try {
      const result = await conn.call("review.fileDiff", { ...pathParams(file), ...scope() });
      if (disposed || rev !== diffRevision) return;
      set({ hunks: result.hunks, diffStatus: "ready" });
    } catch (e) {
      if (disposed || rev !== diffRevision) return;
      const refused =
        e instanceof DaemonError && (e.kind === "ValidationFailed" || e.code === -32008);
      set({ diffStatus: refused ? "withheld" : "error", diffError: errorText(e) });
      if (refused) set("withheld", (items) => [...new Set([...items, fileKey(file)])]);
    }
  }
  async function select(file: FileChange, force = false) {
    ++diffRevision;
    set({
      selected: file,
      hunks: [],
      diffError: undefined,
      diffStatus: file.tooLarge
        ? "withheld"
        : collapsedFile(file) && !force
          ? "collapsed"
          : "loading",
    });
    if (!file.tooLarge && (!collapsedFile(file) || force)) await loadDiff();
  }
  async function refreshChangeset() {
    const rev = ++revision;
    try {
      const [changeset, fullChangeset] = await Promise.all([
        conn.call("review.changeset", { sessionId, ...scope() }),
        state.fromCheckpoint === undefined
          ? Promise.resolve(undefined)
          : conn.call("review.changeset", { sessionId }),
      ]);
      if (disposed || rev !== revision) return;
      if (fullChangeset && changeset.head !== fullChangeset.head) {
        set({
          status: "error",
          error:
            "Review changed while loading its full file inventory. Retry review before shipping.",
        });
        return;
      }
      if (state.sinceLast && changeset.lastReviewed !== state.fromCheckpoint) {
        ++diffRevision;
        set({
          fromCheckpoint: changeset.lastReviewed,
          hunks: [],
          diffStatus: "empty",
          selected: undefined,
        });
        await refreshChangeset();
        return;
      }
      const oldHead = state.changeset?.head;
      const selectedKey = state.selected && fileKey(state.selected);
      set({
        changeset,
        fullChangeset: fullChangeset ?? changeset,
        status: "ready",
        error: undefined,
        stale: conn.state.kind !== "connected",
      });
      if (oldHead !== changeset.head) set("withheld", []);
      if (selectedKey) {
        const file = changeset.files.find((f) => fileKey(f) === selectedKey);
        if (file) {
          set("selected", file);
          if (oldHead !== changeset.head || state.diffStatus === "loading") await select(file);
        } else {
          ++diffRevision;
          set({ selected: undefined, hunks: [], diffStatus: "empty" });
        }
      }
      if (!state.selected && changeset.files[0]) await select(changeset.files[0]);
    } catch (e) {
      if (!disposed && rev === revision) set({ status: "error", error: errorText(e) });
    }
  }
  async function refresh() {
    const sessionRev = ++sessionRevision;
    await Promise.all([
      refreshChangeset(),
      refreshThreads(),
      conn
        .call("session.get", { sessionId })
        .then((session) => {
          if (!disposed && sessionRev === sessionRevision) set("session", session);
        })
        .catch((e) => {
          if (!disposed && sessionRev === sessionRevision)
            set({ status: "error", error: errorText(e) });
        }),
    ]);
  }
  function canShip(head: string | undefined): boolean {
    return (
      head !== undefined &&
      state.status === "ready" &&
      state.changeset?.head === head &&
      state.fullChangeset?.head === head
    );
  }
  function invalidate(all: boolean) {
    refreshAll ||= all;
    if (timer !== undefined) return;
    timer = setTimeout(() => {
      timer = undefined;
      const full = refreshAll;
      refreshAll = false;
      if (disposed || conn.state.kind !== "connected") return;
      void (full ? refresh() : refreshThreads());
    }, 50);
  }
  const sub = conn.subscribe(
    [`session/${sessionId}`],
    (n) => {
      if (n.event.sessionId !== sessionId) return;
      switch (n.event.kind) {
        case "review.thread":
          invalidate(false);
          break;
        case "session.state":
        case "checkpoint":
        case "session.rewound":
        case "turn.started":
        case "turn.ended":
          invalidate(true);
      }
    },
    () => invalidate(true),
  );
  const off = conn.onStateChange((s) => {
    set("stale", s.kind !== "connected");
    if (s.kind === "connected") void refresh();
    else {
      ++revision;
      ++diffRevision;
      ++threadRevision;
      ++sessionRevision;
    }
  });
  if (conn.state.kind === "connected") void refresh();
  async function action<T>(fn: () => Promise<T>): Promise<T> {
    if (state.stale) throw new Error("Disconnected; reconnect before changing this review.");
    set({ busy: true, actionError: undefined });
    try {
      return await fn();
    } catch (e) {
      if (!disposed) set("actionError", errorText(e));
      throw e;
    } finally {
      if (!disposed) set("busy", false);
    }
  }
  return {
    state,
    canShip,
    refresh,
    refreshThreads,
    loadDiff,
    select,
    async setSinceLast(since: boolean) {
      ++diffRevision;
      set({
        sinceLast: since,
        fromCheckpoint: since ? state.changeset?.lastReviewed : undefined,
        selected: undefined,
        hunks: [],
        diffStatus: "empty",
      });
      await refreshChangeset();
    },
    setViewed(file: FileChange, viewed: boolean) {
      return action(async () => {
        await conn.call("review.setViewed", { ...pathParams(file), viewed });
        await refreshChangeset();
      });
    },
    comment(range: { startLine: number; endLine: number }, body: string) {
      return action(async () => {
        if (!state.selected) throw new Error("Select a file first");
        await conn.call("review.comment", { ...pathParams(state.selected), range, body });
      });
    },
    reply(threadId: string, body: string) {
      return action(() => conn.call("review.reply", { threadId, body }));
    },
    resolve(threadId: string) {
      return action(() => conn.call("review.resolve", { threadId }));
    },
    requestChanges() {
      return action(() => conn.call("review.requestChanges", { sessionId }));
    },
    revert(hunkId: string) {
      return action(async () => {
        if (!state.selected) throw new Error("Select a file first");
        await conn.call("review.revertHunk", { ...pathParams(state.selected), ...scope(), hunkId });
      });
    },
    ship(checkpointId: string, strategy: ShipStrategy, branch: string, includeWithheld: boolean) {
      return action(async () => {
        if (!canShip(checkpointId))
          throw new Error("Review is stale or inconsistent; reload before shipping.");
        return conn.call("review.ship", {
          sessionId,
          checkpointId,
          strategy,
          ...(strategy === "push-branch" && branch ? { branch } : {}),
          ...(includeWithheld ? { includeWithheld: true } : {}),
        });
      });
    },
    dispose() {
      disposed = true;
      ++revision;
      ++diffRevision;
      ++threadRevision;
      ++sessionRevision;
      clearTimeout(timer);
      off();
      sub.unsubscribe();
    },
  };
}
export type ReviewStore = ReturnType<typeof createReviewStore>;
