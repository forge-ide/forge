# Review end-to-end checks

`just test-e2e` uses real `forged` and `forge-fake-agent` binaries, a private
short-lived git repository and explicit temporary Forge data/runtime/config/HOME
paths. The fixture git identity does not depend on a developer's git settings.
The agent YAML at `tests/fixtures/agents/review-loop.yaml` writes whole files and
answers through the real `review_reply` MCP endpoint. No real model is used.

On Linux, install WebKitWebDriver and Xvfb, then run `just e2e-drivers` followed
by `GIT_CONFIG_GLOBAL=/dev/null just test-e2e`. On current Arch, the system
packages are `webkitgtk-6.0`, `xorg-server-xvfb` and, if absent, `xorg-xauth`.
Ubuntu CI uses `just e2e-system-deps`. The Linux adapter starts its driver under xvfb-run when
DISPLAY is absent, with its normal Xauthority authentication and TCP disabled.
It drives the real Tauri application with WebdriverIO and pinned tauri-driver
2.1.0; it does not replace the Linux app with a browser build. WebDriver requests have a bounded 60-second transport timeout, including native session creation; launch timing/version evidence is recorded in `launch.json`. The browser probes retain their own deadlines and the exact pane-opening/scrolling limits.

On macOS, `just e2e-drivers` installs the pinned Playwright WebKit browser, and
`just test-e2e` builds the browser host and runs WebKit. Every macOS result is a
**Playwright WebKit proxy**, not the native Tauri WKWebView. The loopback bridge
lives only in this test directory; it requires the exact test Origin and a random
per-run capability and forwards only to the fixture Unix socket. `forged` has
no TCP listener. The normal desktop build never imports the browser host.

The UI loop creates a persistent comment, requests changes, observes the fake
agent's reply and resolves the thread. RPC verifies persisted state after UI
interaction; it does not replace the review mutations.

The opening benchmark changes exactly 5,000 lines across five 1,000-line files,
which legitimately expand by default. Timing begins at session-button activation
and includes RPC, rendering and a subsequent animation-frame paint opportunity
with visible non-empty code. Five fresh pane openings produce raw samples; their
median must be **strictly less than 400 ms**.

Scrolling traverses every file in that changeset. A second exactly 5,000-line
single-file fixture first verifies default collapse, explicitly clicks Load diff,
and then scrolls all 5,000 lines. Every consecutive rAF delta from scroll movement
through the final painted code is retained; **the maximum must be <=32 ms**.
The final line must be visible, scrolling must reach the end, and the code DOM
must remain virtualized. There is no filtering of long frames or automatic retry
of a failed measurement. Ask the owner before changing any target.

Platform-labelled raw JSON, screenshots and daemon/driver failure logs are saved
under `target/e2e-artifacts/` and uploaded by both CI jobs. Raw measurements are written before threshold assertions, including on failure; the current platform output is cleared before each invocation so old successes cannot masquerade as current results. SIGINT/SIGTERM handlers await cleanup, including startup in progress; owned driver/display/daemon processes use isolated process groups. Shutdown waits for the group, including descendants after a leader exits, and escalates to SIGKILL after the grace period. A capacity failure
without a runner is not a test pass. Do not claim the milestone complete until
both required platform checks actually pass.

`just test-e2e-unit` runs transport/build-isolation, bridge authorization and
measurement-boundary tests; `just ci` includes these unit checks. The actual
browser flow remains `just test-e2e`, in its own Linux/macOS CI matrix.

New test dependencies are pinned: WebdriverIO 10.1.0 (MIT, actively maintained
upstream) drives native WebDriver; Playwright 1.64.1 (Apache-2.0, Microsoft-maintained)
provides the macOS WebKit proxy; ws 8.22.1 (MIT, actively maintained upstream)
provides the test-only WebSocket bridge and Unix client. tauri-driver 2.1.0 is an
Apache-2.0 OR MIT tool maintained with Tauri. Optional Edge/Gecko install scripts
remain explicitly denied by pnpm policy; the harness does not use those drivers.
