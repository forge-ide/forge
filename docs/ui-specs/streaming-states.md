# Streaming States

> Extracted from SPECS.md §11 and §14 — streaming cursor, monitor pulse ring, provider live-dot glow, transitions, and complete motion reference

---

## 11. Streaming states

Every motion in the UI communicates state. The streaming cursor and the agent-monitor pulse ring are the only two standing animations; everything else is transition-only.

### 11.1 Streaming cursor

- Size: 5×12px, radius 1px
- Color: `--color-ember-400`
- Blink: 1s period, 50% duty (`steps(1,end)`)
- Position: at the end of the currently-streaming text run
- Never present on completed messages — removed the moment streaming ends
- Never used in editors, terminals, or file panes — chat is the only surface that streams this way

### 11.2 Monitor pulse ring

- Position: around the dot of the currently-running step
- 1px border, `--color-warn`
- Keyframe: `scale(1) opacity(1)` → `scale(2) opacity(0)` over 1.5s infinite
- Used only in agent monitor trace timeline

### 11.3 Provider live-dot glow

- Glow shadow on the 7px semantic dot when the provider is *live connected*
- `box-shadow: 0 0 6px rgba(<semantic>, 0.5)`
- Static, not animated — connection state is binary

### 11.4 Transitions
- All state transitions: 0.15s ease (DESIGN.md `--ease`)
- No easing variation, no custom cubic-bezier curves
- No stagger animations

---

## 14. Motion reference

The complete list of allowed motion in Forge. Anything not on this list should not be added without updating this document.

| Motion | Where | Duration | Curve | Purpose |
|---|---|---|---|---|
| Generic transition | Hover/focus/active state changes | 0.15s | `ease` | State feedback |
| Streaming cursor | Chat (during stream only) | 1s loop | `steps(1,end)` | Streaming state |
| Monitor pulse ring | Currently-running trace step | 1.5s loop | `ease-out` | Running state |
| Session state pulse | Dashboard "streaming" state indicator | 1.6s loop | `ease-in-out` | Live session |
| Tool card expand | On click | 0.15s | `ease` | Disclosure |
| Composer border focus | On focus | 0.15s | `ease` | Focus state |
| Toast enter | Appearance | 0.2s | `ease-out` | Notification |
| Drag-dock drop zone | Pane drag active | 0.1s | `ease` | Drop target highlight |
| Tab switch | No animation | 0s | — | Instant |
| Pane resize | No animation | 0s | — | Instant |
| View change | No animation | 0s | — | Instant |

**Explicitly disallowed.**
- Entry/exit animations for anything that isn't a toast or modal
- Parallax or scroll-linked effects
- Hover "lift" (box-shadow growth) on cards or buttons
- Skeleton shimmer loaders (we show real data streaming, not placeholders)
- Spring / physics curves
- Number counters that tick up
- Gradient shifts that don't communicate state
