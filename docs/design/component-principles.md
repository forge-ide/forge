# Component Principles

> Extracted from DESIGN.md §6 — buttons, inputs, toasts, status bar, and code blocks

**Related docs:** [Color System](color-system.md) · [Token Reference](token-reference.md) · [Typography](typography.md)

---

## 6. Component Principles

### Buttons

Four variants: **primary**, **secondary**, **ghost**, **icon**. Color names below (`ember-400`, `iron-600`, etc.) are defined in the [Color System](color-system.md) and exposed as CSS custom properties in [Token Reference](token-reference.md).

- **Primary** (`ember-400` fill): one per view maximum. The most important action.
- **Secondary** (ember outline): alternative actions, same importance as primary but not the default.
- **Ghost** (neutral border): destructive alternatives, cancel, skip.
- **Icon** (neutral border): toolbar actions, panel controls.

All buttons use Barlow Condensed 700, uppercase, `letter-spacing: 0.1em`. Active state always includes `transform: translateY(1px)`.

Disabled buttons use `iron-600` background and text. Never reduce opacity on a button to show disabled state — opacity makes elements appear interactive.

### Inputs

- Default border: `iron-600`
- Hover border: `iron-300`
- Focus border: `ember-400` — always, without exception
- Error border: `ember-400` (same as focus — the context makes the meaning clear)
- Success border: `green` (`#3ddc84`)

Labels use `mono-xs` style: Fira Code, 9px, uppercase, `letter-spacing: 0.2em`, `iron-300` color.

### Toasts

Toasts have a 3px left accent bar (the semantic color), a dark tinted background, and a semantic border. They stack from the bottom-right, above the status bar, maximum 4 visible.

- Success: auto-dismiss 5s
- Info: auto-dismiss 5s
- Warning: auto-dismiss 8s
- Error: persists until actioned

### Status bar

The status bar is always `ember-400` background with white text. This is the most visible brand surface in daily use. It always shows: the Forge mark, active provider, streaming state, and file context. Do not change the status bar color under any circumstances, including in light mode.

### Code blocks

Code blocks use `#050709` background (slightly darker than `iron-900`) to create depth. The header bar shows language and copy/insert actions. Highlighted lines use a left border of `ember-400` with `rgba(255,74,18,0.07)` background.
