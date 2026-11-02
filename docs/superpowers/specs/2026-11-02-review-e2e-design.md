# Review end-to-end and performance design

## Intent and approved architecture

Prove the review loop through the UI against a real temporary `forged` and
`forge-fake-agent`, and enforce the exact review-pane performance targets.
The review-pane PR is merged at `051574915a0`; this work is isolated on
`feat/review-e2e` in `.worktrees/review-e2e`.

The user already approved the two-platform approach: Linux uses WebdriverIO,
`tauri-driver` and WebKitWebDriver against the real Tauri application; macOS
uses Playwright WebKit against a browser build via `host/web.ts`, with a
Node loopback WebSocket-to-Unix-socket bridge confined to `tests/e2e`.
Never add a TCP listener to `forged`. The macOS measurement is explicitly a
proxy for the real Tauri WKWebView, not a native-webview result.

Keep the existing exact limits: first file visible in less than 400 ms,
median of five openings, and maximum rAF delta at most 32 ms while scrolling.
Do not weaken, round away, skip or retry away a failing performance assertion.

## Approaches

A common mocked browser test would not exercise daemon persistence, the
Forge MCP reply endpoint, or Linux's real webview. It is ruled out by the
approved harness. A native macOS WebDriver cannot cover WKWebView through
`tauri-driver`; the user selected Playwright WebKit as the proxy. Use a
shared flow/measurement contract with small Linux and macOS browser adapters,
so the assertions and fixture semantics stay identical across platforms.

## Fixture and flow

Start real built binaries and a fresh git repository under a short private
temporary root. Set `FORGE_DATA_DIR`, `FORGE_RUNTIME_DIR`, `FORGE_CONFIG_DIR`,
`FORGE_MANAGED_POLICY` and fixture HOME explicitly. Use an isolated git identity
for fixture commits and also run all tests with `GIT_CONFIG_GLOBAL=/dev/null`.
Do not touch real user data, sockets, agents, repositories or credentials.

Seed a project and session through the existing Unix-socket JSON-RPC API;
there is no New session UI yet. After setup, the browser opens the project
session and drives: select new-side line/range, save comment, Request changes,
observe the agent reply, Resolve, and verify the persisted resolved thread.
Check request-changes changes the round and the reply author is `agent`.

Commit a YAML fake-agent script using whole-file `write` content. Existing
`expect_review_comments` and `reply_thread` support the second turn;
`reply_thread` calls `review_reply` on the session's Forge MCP endpoint.
No `patch` support or real agent is needed. Use the existing test-profile
pattern with explicitly allowed Unsandboxed fake-agent sessions; sandbox
security coverage remains the separate required suite.

## Browser boundary and bridge

Extract the transport's type-only contract if needed, keeping all Tauri
imports inside host/. The normal build keeps the Tauri transport. A separate
explicit browser-test build mode resolves a host factory to `host/web.ts`.
It must not make the normal app accept network endpoints or add a browser
fallback. Desktop production assets and test browser assets have separate
output directories.

The browser transport talks only to the current test page's loopback origin.
The harness supplies a random per-run capability in its URL; the transport
builds a same-origin WebSocket URL, never a user-supplied arbitrary daemon host.
Connection replacement, send-before-connect, errors, close and reconnect obey
DaemonTransport's existing contract and receive unit tests.

The Node bridge listens only on 127.0.0.1 with an ephemeral port. It opens only
the fixture's preselected Unix socket, never a path chosen by an HTTP request.
Require the exact expected Origin and random capability for every upgrade;
reject missing or wrong credentials, paths and origins. Bound frame sizes and
connection lifetime, propagate closes both ways, and close all connections on
shutdown. Serve only built test UI assets under their fixed root, without
path traversal or a directory listing. The server is test-only code and is
never packaged into `forged` or the desktop app.

## Performance contract

Use actual `performance.now()` and animation-frame callbacks inside the tested
webview. A mutation observer can signal rendered content, but timing completion
must include a subsequent paint opportunity and visible non-empty code rows,
not merely a file header, loaded data or hidden DOM.

Use a 5,000-added-line changeset spread over five 1,000-line files. This avoids
bypassing the required >1,500-line default-collapse behavior. Measure from the
session-open UI activation until the first selected file's actual code is
visible. Repeat five fresh pane openings; preserve all five raw timings and
assert their median is strictly below 400 ms. Do not subtract RPC time.

Scroll each file in that same changeset through its entire CodeMirror range,
recording every consecutive rAF delta throughout each scroll interval. Assert
the maximum is <=32 ms. Verify meaningful scroll progress, the final lines,
and viewport-bounded DOM so a stalled or unrendered test cannot pass. Use
continuous programmatic scrolling driven by rAF, independent of driver polling.
Do not filter out long frames inside the measured interval.

Also exercise a separate single-file 5,000-line changeset: assert it initially
collapses, click Load diff explicitly, then apply the same full-range scrolling
and maximum-frame assertion to all 5,000 lines. This adds single-file stress
coverage without redefining the first-opening target or disabling collapse.
Use foreground/headed rendering where needed for meaningful rAF cadence.

Keep platform-labelled JSON measurements, browser versions, screenshots and
failure logs as CI artifacts. Linux results identify real Tauri/WebKitGTK;
macOS reports always say Playwright WebKit proxy.

## Commands, CI and dependencies

Replace the `just test-e2e` stub with build/setup/run recipes. CI adds a separate
end-to-end job on Linux and macOS, leaving existing checks in place. Linux
installs the WebKit driver and Xvfb, with pinned `tauri-driver`; macOS installs
the pinned Playwright WebKit browser. Use just recipes in CI, not direct cargo
or pnpm commands. Runner-capacity failures remain infrastructure failures,
never a successful or skipped platform result.

Expected development dependencies: WebdriverIO, Playwright and `ws`; verify and
pin versions, license and active maintenance before installation, and document
all additions in the PR description. Prefer Node's built-in test runner and
existing workspace tooling over adding a second test framework or YAML parser.
No forge-api, config-schema or migration changes are expected.

## Validation and completion

Write failing tests first for the browser transport, bridge authorization and
cleanup, measurement median/boundaries and fake-agent UI loop. Mutation-check
bridge Origin/capability enforcement, exact performance thresholds, and the
reply/resolved assertions so the suite cannot falsely pass missing behavior.
Preserve review-pane security tests from the first PR.

Run `just gen`, `GIT_CONFIG_GLOBAL=/dev/null just ci`, the real Linux end-to-end
suite and then `just test-sandbox`. Obtain independent security review and ask
the same GPT-6 Astra adviser for each security finding; re-review fixes. Bring
scope/security-default changes or any proposed target relaxation to the user.

Do not mark the milestone done until the UI flow and exact performance checks
have actually passed on both required platforms. This machine is Linux;
macOS coverage comes from CI and remains pending until the user authorizes
publishing this branch. Do not commit, push or open the second PR until told.
Moving session worktrees, squash shipping, Git LFS notes and accept hunk remain
out of scope.
