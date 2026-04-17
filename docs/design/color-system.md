# Color System

> Extracted from DESIGN.md §3 — Ember brand scale, Iron surface scale, semantic colors, Steel accent, token usage rules, and agent coding rules

---

## 3. Color System

### Palette overview

Forge uses two base scales (Ember and Iron) and a set of semantic tokens. Always use tokens in code — never raw hex values except when defining the tokens themselves.

### Ember scale — brand

The primary brand scale, ranging from near-black heat to warm cream. `ember-400` is the brand color.

| Token | Hex | Usage |
|---|---|---|
| `ember-900` | `#2a0800` | Brand-tinted backgrounds, hover wash |
| `ember-500` | `#cc3a00` | Pressed state on brand buttons |
| `ember-400` | `#ff4a12` | **Primary brand. CTAs, active indicators, logo** |
| `ember-300` | `#ff7a30` | Hover state, secondary ember accents, syntax keywords |
| `ember-200` | `#ffaa33` | Gradient ends, amber warnings, syntax operators |
| `ember-100` | `#ffd166` | Gold accents, hub node, syntax function names |
| `ember-50`  | `#fff4d6` | Lightest tint, rarely used |

### Iron scale — surfaces and text

The UI grey scale. Every surface, border, and secondary text color comes from here.

| Token | Hex | Usage |
|---|---|---|
| `iron-900` | `#07080a` | App background |
| `iron-850` | `#0d0f13` | Panels, sidebars |
| `iron-800` | `#13161d` | Tab bar, cards, dropdowns |
| `iron-750` | `#181c26` | Hover states, selected rows |
| `iron-700` | `#1c2230` | Default borders, dividers |
| `iron-600` | `#252f3e` | Focus borders, selected borders |
| `iron-500` | `#3a4558` | Tertiary text, inactive items |
| `iron-300` | `#6a7e98` | Secondary text, descriptions |
| `iron-200` | `#8a9aac` | Labels, hints |
| `iron-100` | `#b8c8d8` | Light accents |
| `text`     | `#eae6de` | Primary text — warm off-white, not pure white |

> **Why warm off-white?** Pure `#ffffff` on `#07080a` produces too much contrast and eye strain over long sessions. The warm `#eae6de` is intentional and should not be changed to white.

### Semantic colors

Each semantic color has four shades: dark (background tint), mid (border), base (text/icon), light (on-dark text).

| Semantic | Base | Usage |
|---|---|---|
| Success | `#3ddc84` | Connected, completed, written, saved |
| Warning | `#ffaa33` | Approaching limits, degraded state |
| Error | `#ff4a12` | Failed, unreachable, invalid — same as ember-400 |
| Info | `#7aaaff` | Updates, links, references |

### Steel — accent

`#7aaaff` is used for links, info states, and the local/llama accent in multi-provider contexts. It is the only blue in the palette. Do not introduce other blues.

### Design token usage rules

```css
/* ✓ Correct — use tokens */
color: var(--color-text-primary);
background: var(--color-surface-2);
border-color: var(--color-border-brand);

/* ✕ Wrong — never raw hex in component code */
color: #eae6de;
background: #13161d;
```

### Color rules for agents

When writing or modifying UI code:
- Background surfaces must always come from the Iron scale in order: `iron-900` → `iron-850` → `iron-800` → `iron-750` (deepest to most elevated)
- Never use a lighter surface below a darker one
- Active/selected states use `iron-750` background + `ember-400` left border or underline
- Disabled states use `iron-600` text on `iron-800` background — never reduce opacity on interactive elements
- Error states use ember-400, not a different red
