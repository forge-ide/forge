# Error handling

How Forge crates choose between `thiserror` (typed enums) and `anyhow` (boxed
contextual errors), and what that means for callers.

## Policy

Pick per-crate based on the failure taxonomy at the public boundary:

- **Typed enum (`thiserror`)** when the failure set is small, closed, and the
  *caller will branch on the variant*. The point is to let downstream code
  pattern-match instead of parsing `Display` strings.
- **`anyhow::Error`** when the failure set is heterogeneous (HTTP + IO + JSON
  + transport-level codec) and the caller almost always either propagates or
  surfaces the error to a human. Adding `#[from]` impls for every external
  source crate is high churn for no caller value.

The choice is per crate, not per call site — mixing the two inside one crate
forces consumers to handle both shapes for the same domain, which defeats
either approach.

## Current crates

| Crate | Boundary type | Why |
|-------|---------------|-----|
| `forge-fs` | `mutate::FsError` (`thiserror`) | Closed taxonomy: `PathDenied`, `SymlinkDenied`, `ParentMissing`, `TargetMissing`, `MalformedPatch`, `TooLarge`, `Io`. The dispatcher branches on the variant to map each one to the right user-visible event (e.g. `PathDenied` → approval-prompt rephrase, `Io` → retryable). |
| `forge-session` | `error::SessionError` (`thiserror`) | Closed taxonomy at the orchestrator boundary: `EventLogAppend`, `EventLogFlush`, `ByteBudgetExceeded`. Established in F-076 to let the server loop distinguish retryable I/O from fatal state without parsing `Display`. See `crates/forge-session/src/error.rs`. |
| `forge-providers` | `forge_core::Result` (boxed `anyhow::Error`) | Heterogeneous: `reqwest` transport, `serde_json` decode, `tokio_util::codec::LinesCodec` framing, plus typed `ChatChunk::Error { kind: StreamErrorKind, .. }` for terminal stream-level failures. The terminal kinds are typed (so the dashboard can render them); the construction-time and request-time errors are boxed. |
| `forge-shell` | `Result<T, String>` at the Tauri command boundary | Tauri requires `serde::Serialize` on the error; `String` is the lowest-friction shape. Internal helpers can use `anyhow` and `.map_err(|e| e.to_string())` at the boundary. |

## Practical guidance

1. **Pick at the crate boundary, not the call site.** A crate's *public*
   functions should all return the same error shape. Internal helpers can use
   whatever is convenient and convert at the boundary.

2. **A typed boundary outranks an `anyhow` one for the same failure surface.**
   If you need to add a single new variant to a `thiserror` enum to keep a
   match exhaustive, do that — don't reach for `anyhow` to avoid the churn.

3. **Don't wrap a typed error in `anyhow!` if you control both sides.** This
   was the F-076 finding: `Session::emit` was wrapping `forge_core::ForgeError`
   in `anyhow::anyhow!(e)` and erasing the variant. Either propagate the typed
   error or convert to the parent's typed enum — `anyhow!` is for *foreign*
   errors that have no typed shape worth preserving.

4. **Terminal vs in-band errors are both fine to type independently.**
   `forge-providers` returns `anyhow::Error` from `chat()` itself (request
   construction can fail in N ways) but exposes typed `StreamErrorKind` on
   `ChatChunk::Error` for the in-band stream-failure case. Same crate, two
   shapes — but each is uniform within its lane.

5. **`#[source]` over `String` for inner errors.** When wrapping, prefer
   `#[source]` so `std::error::Error::source()` walks the chain. The
   `SessionError` enum uses this pattern (`EventLogAppend(#[source] ForgeError)`).

## When to convert a crate from `anyhow` to typed

Trigger: a downstream consumer is matching on `Display` strings to branch
behavior. That is the signal that the failure taxonomy is closed enough to
deserve an enum, and you are paying for `anyhow`'s flexibility without using
it. Convert; the consumer's parsing code becomes a `match` and the enum
absorbs the closed set.

Counter-trigger: the error set is genuinely open (random transport library
errors, third-party serialization, IO from N filesystems). Stay with `anyhow`
and add `.with_context(...)` at each layer so the chain reads.
