# DESIGN.md

> **For humans and agents:** This document defines the design principles, visual language, and decision-making framework for Forge IDE. Read this before making any UI change, adding a component, or modifying a color. When in doubt, refer back to the principles in section 1.

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Visual Identity](#2-visual-identity)
3. [Color System](#3-color-system)
4. [Typography](#4-typography)
5. [Spacing & Layout](#5-spacing--layout)
6. [Component Principles](#6-component-principles)
7. [AI-Specific UI Patterns](#7-ai-specific-ui-patterns)
8. [Writing & Voice](#8-writing--voice)
9. [Do and Don't](#9-do-and-dont)
10. [Token Reference](#10-token-reference)

---

## 1. Design Philosophy

### The one sentence

**Forge is a tool, not a product. Design it like one.**

Everything in the UI should earn its place by being useful. Decoration without function is a bug. Animations that don't communicate state are noise. Colors that don't carry meaning are waste.

### Three principles

**Industrial utility.** Forge is a forge — a place where serious work happens. The aesthetic should reflect that: dense, precise, dark, functional. No rounded corners on load-bearing UI. No gradients that don't encode information. No empty space added for "breathing room" when that space could show the user something useful.

**Transparent by default.** Everything the AI does is visible. Tool calls, token counts, agent steps, streaming state — all of it surfaced, none of it hidden. The UI never obscures what the system is doing. Trust is built through visibility.

**Parity between AI and code.** The chat pane and the code pane are peers. Neither is a sidebar. Neither is secondary. Layout decisions should reflect this: when space is allocated, AI and code get equal consideration.

### What Forge is not

- It is not a consumer product. Don't design for delight. Design for efficiency.
- It is not a demo. Don't add animations because they look impressive. Add them because they communicate state.
- It is not VS Code with a chat plugin. Every design decision should reinforce that this is a genuinely different product with a genuinely different model.

---

## 2. Visual Identity

### The mark

The Forge mark is a hub-and-spoke diagram with a minimal flat hammer at the center. It represents the network of AI providers connecting to a single forge point.

**Never:**
- Rotate the mark
- Change spoke colors independently
- Add drop shadows or glows to the mark
- Use the mark at sizes below 16×16px
- Use the wordmark without the mark in product contexts

**Construction:**
- 5 spokes radiating from center hub
- Top spoke: `#ffd166` (gold)
- Upper two spokes: `#ff4a12` (ember)
- Lower two spokes: `#ff7a30` (ember-dim)
- Endpoint nodes: open circles (fill = background, stroke = spoke color)
- Hub: dark circle `#0c1018` with stroke `#1e2838`
- Hammer: steel head `#4a5668`, polished bevel `#8a9aac`, ember strike stripe `#ff4a12` → `#ffaa33`, dark handle `#2a2018`

### Wordmark

- Font: Barlow Condensed 900 weight
- Always uppercase: `FORGE IDE`
- Letter spacing: 0.04em on the wordmark, 0.16em in display contexts
- Tagline: `"Any AI. One editor."` in Fira Code, 9px, `#6a7e98`

### Colorways

| Context | Background | Usage |
|---|---|---|
| Dark (primary) | `#07080a` | All in-app UI |
| Light (alternate) | `#f0ede8` | Light mode |
| Ember (reversed) | `#ff4a12` | Status bar, CTAs on dark backgrounds |
| Monochrome | `#13161d` | Single-color contexts |

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

---

## 4. Typography

### Type families

| Role | Family | Usage |
|---|---|---|
| Display | Barlow Condensed | Headings, UI chrome, wordmark, marketing |
| Body | Barlow | Prose, descriptions, menu items, docs |
| Mono | Fira Code | Code, paths, shortcuts, identifiers, section labels |

### Rules — Barlow Condensed (Display)

- **Always uppercase.** Never sentence case. Never title case.
- Weight 900 for hero/marketing headlines
- Weight 800 for section headers, panel titles
- Weight 700 for dialog titles, secondary headers
- Minimum size: 14px
- Never use for body copy, descriptions, or flowing text

### Rules — Barlow (Body)

- **Always sentence case** for prose and descriptions
- Weight 400 for body copy and menu items
- Weight 500–600 for toast titles and emphasis
- Weight 300 for captions and secondary metadata
- Italic only for inline emphasis — use sparingly
- Minimum size: 12px

### Rules — Fira Code (Mono)

- All code, inline and block
- File names, paths, extensions
- Keyboard shortcuts and keybindings
- Error codes, status identifiers
- Section labels at 9px with `letter-spacing: 0.3em` and `text-transform: uppercase`
- Never for flowing prose
- Minimum size: 9px
- Enable ligatures — `font-feature-settings: "liga" 1, "calt" 1`

### Type scale

| Token | Size | Weight | Family | Usage |
|---|---|---|---|---|
| `display-2xl` | 72px | 900 | Condensed | Hero headlines, splash |
| `display-xl` | 48px | 800 | Condensed | Marketing section headers |
| `display-lg` | 32px | 700 | Condensed | Feature headings, modal titles |
| `display-md` | 22px | 700 | Condensed | Panel headers, dialog titles |
| `body-lg` | 16px | 400 | Barlow | Marketing body, onboarding |
| `body-md` | 14px | 400 | Barlow | Descriptions, docs, tooltips |
| `body-sm` | 12px | 400 | Barlow | UI menu items, toast messages |
| `mono-md` | 13px | 400 | Fira Code | Code editor, inline code |
| `mono-sm` | 11px | 400 | Fira Code | Tab names, file paths, shortcuts |
| `mono-xs` | 9px | 400 | Fira Code | Panel section labels, badges |

### Fira Code ligatures in use

These ligatures should be enabled anywhere Fira Code is used: `!=` `==` `=>` `->` `>=` `<=` `::` `...`

---

## 5. Spacing & Layout

### Spacing scale

Forge uses a base-4 spacing scale. All spacing values should come from this scale.

| Token | Value | Usage |
|---|---|---|
| `sp-1` | 4px | Icon padding, tight component gaps |
| `sp-2` | 8px | Internal component padding (sm) |
| `sp-3` | 12px | Chat input padding, compact items |
| `sp-4` | 16px | Standard panel padding |
| `sp-5` | 20px | Card padding, modal body padding |
| `sp-6` | 24px | Section gaps within panels |
| `sp-8` | 32px | Card padding, major component spacing |
| `sp-10` | 40px | Large layout gaps |
| `sp-12` | 48px | Section headers, hero padding |

### Border radii

| Token | Value | Usage |
|---|---|---|
| `r-sm` | 3px | Buttons, inputs, badges, chips, code blocks |
| `r-md` | 5px | Toasts, dropdowns, panels |
| `r-lg` | 8px | Cards, modals, shell containers |

> **Design principle:** Forge uses small radii deliberately. Large border radii (12px+) signal softness and consumer product aesthetics. The 3px default reads as precise and utilitarian. Do not increase these.

### Layout hierarchy — shell structure

```
Window
├── Title bar (32px)
├── Body
│   ├── Activity bar (40px wide)
│   ├── Sidebar panel (190px default, resizable)
│   └── Main canvas (flex: 1)
│       ├── Tab bar (33px)
│       └── Quad canvas (grid: 1fr 1fr / 1fr 1fr)
│           ├── Pane TL
│           ├── Pane TR
│           ├── Pane BL
│           └── Pane BR
└── Status bar (22px) — always ember background
```

### Surface elevation order

Surfaces must always stack from dark (deep) to light (elevated). Never violate this order.

```
iron-900 (bg)        ← deepest — app background
iron-850 (surface-1) ← panels, sidebars
iron-800 (surface-2) ← tab bar, cards, dropdowns
iron-750 (surface-3) ← hover states, selected items
iron-700 (border-1)  ← borders and dividers
iron-600 (border-2)  ← focused borders
```

---

## 6. Component Principles

### Buttons

Four variants: **primary**, **secondary**, **ghost**, **icon**.

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

---

## 7. AI-Specific UI Patterns

These patterns are Forge-specific. Follow them consistently — they are part of the product's identity.

### Provider identity

Each AI provider gets an accent color used consistently across all panes:
- Anthropic / Claude: `ember-400` (`#ff4a12`)
- OpenAI / GPT: `amber` (`#ffaa33`)
- Local / Ollama / LM Studio: `steel` (`#7aaaff`)
- Custom endpoints: `iron-200` (`#8a9aac`)

The provider dot, send button, pane header indicator, and agent thread accent all use this color. When a user opens a pane, the color instantly communicates which model is active.

### Streaming state

Streaming responses show a blinking cursor: `width: 5px`, `height: 12px`, `background: ember-400`, `border-radius: 1px`, blinking at 1s intervals (50% duty cycle). This cursor is always present during streaming and disappears the moment streaming ends. It should never appear on a completed message.

### Tool call visibility

Tool calls surface as inline collapsible cards within AI message bubbles:
- Icon: `ember-100` (gold) for tool calls, `steel` for resource reads, `ember-400` for agent spawns
- Background: `rgba(255,209,102,0.04)` with `rgba(255,209,102,0.15)` border
- Always show: tool name, arguments (truncated if long), result, duration
- Always collapsible — expanded by default for the most recent call, collapsed for older ones

### Sub-agent banners

When an orchestrator spawns a sub-agent, it appears as a banner within the parent chat thread:
- Left icon: colored by the spawning model's accent color
- Title prefix: always `↳ spawned sub-agent: [name]`
- Status chips: model name, tool count, running/done/queued/error state
- Sub-thread: indented with a `2px border-left` in `iron-600`, collapsible

### Agent monitor

The agent monitor shows a list of all agents with a progress bar (3px height, `iron-800` track) and a step-by-step trace on selection. Progress bars use semantic colors: green for done, amber for running, iron-500 for queued, ember for error.

### MCP servers

MCP servers in the panel show a 7px status dot with a glow shadow when connected: `box-shadow: 0 0 6px rgba(61,220,132,0.5)`. This glow is the only glow used in the UI — it specifically communicates live network connectivity, which is a meaningfully different state than simple "active".

---

## 8. Writing & Voice

### Voice principles

Forge copy is: **Direct. Precise. Technical. Confident. Terse.**

It is not: Casual, vague, simplified, apologetic, or verbose.

### Formatting rules for UI copy

- Error messages: state what happened, state the cause, offer the action. Three sentences maximum. Often one.
- Status indicators: noun + state, no verbs. `anthropic · streaming` not `Claude is currently generating a response`.
- Button labels: verb + noun in display caps. `CONNECT PROVIDER` not `Connect to your AI provider`.
- Section labels: noun only in mono uppercase. `PROVIDERS` not `Your AI Providers`.
- Toast titles: state + subject. `AI backend connected` not `Successfully connected to your AI backend`.

### Terminology

Always use these terms, never the alternatives:

| Use | Never |
|---|---|
| AI provider / backend | AI model, LLM (in UI) |
| MCP server | Plugin server, tool server |
| Sub-agent | Child AI, helper, assistant |
| Workspace | Project, session folder |
| Tool call | Function call, API call |
| Streaming | Live, real-time |
| Pane | Window, panel (for canvas divisions) |
| Canvas | Editor area, main area |

### Numbers and codes

Always show technical identifiers verbatim: `ECONNREFUSED 127.0.0.1:11434` not `connection refused`. Developers trust exact errors more than plain-English rewrites.

---

## 9. Do and Don't

### Visual

| ✓ Do | ✕ Don't |
|---|---|
| Use ember-400 for one primary action per view | Use ember for decorative accents |
| Use the iron scale for all surfaces | Introduce new greys outside the iron scale |
| Keep border radii at 3px for most components | Increase radii to "soften" the feel |
| Show tool calls inline in chat | Hide AI actions in a separate log only |
| Use the ember status bar always | Change the status bar color in any theme |
| Show streaming with the blinking cursor | Use a spinning loader for streaming |
| Use Barlow Condensed uppercase for headings | Mix casing styles in the same context |

### Behaviour

| ✓ Do | ✕ Don't |
|---|---|
| Surface every AI action in the tool call log | Silently execute tool calls |
| Show provider connection state in the sidebar | Assume the user knows what's connected |
| Use error toasts that persist until actioned | Auto-dismiss errors |
| Label every pane with its active provider | Leave panes without provider context |
| Keep the @ context system consistent | Create different context systems per pane |

### For agents making code changes

When an AI agent is modifying Forge UI code, it must:

1. **Use design tokens**, never raw hex or pixel values
2. **Check surface elevation order** — no surface should be lighter than the one above it
3. **Preserve ember for active states** — if you're adding an active/selected state, the indicator color is `ember-400`
4. **Match existing component patterns** — before creating a new component, check if an existing one can be extended
5. **Never add animations** unless they communicate state change (not for decoration)
6. **Never change the status bar color** — it is always `ember-400`
7. **Never introduce new font families** — the three families are locked

---

## 10. Token Reference

### CSS custom properties

These tokens should be defined at `:root` and used throughout. Never use raw values in component styles.

```css
:root {
  /* Ember brand scale */
  --color-ember-900: #2a0800;
  --color-ember-500: #cc3a00;
  --color-ember-400: #ff4a12;   /* primary brand */
  --color-ember-300: #ff7a30;
  --color-ember-200: #ffaa33;
  --color-ember-100: #ffd166;

  /* Iron surface scale */
  --color-bg:         #07080a;
  --color-surface-1:  #0d0f13;
  --color-surface-2:  #13161d;
  --color-surface-3:  #181c26;
  --color-border-1:   #1c2230;
  --color-border-2:   #252f3e;

  /* Text */
  --color-text-primary:   #eae6de;
  --color-text-secondary: #8a9aac;
  --color-text-tertiary:  #3a4558;
  --color-text-disabled:  #252f3e;
  --color-text-ember:     #ff4a12;
  --color-text-link:      #7aaaff;

  /* Semantic */
  --color-success:   #3ddc84;
  --color-warning:   #ffaa33;
  --color-error:     #ff4a12;
  --color-info:      #7aaaff;

  /* Semantic backgrounds */
  --color-success-bg: rgba(61,220,132,0.07);
  --color-warning-bg: rgba(255,170,51,0.07);
  --color-error-bg:   rgba(255,74,18,0.07);
  --color-info-bg:    rgba(122,170,255,0.07);

  /* Semantic borders */
  --color-success-border: rgba(61,220,132,0.22);
  --color-warning-border: rgba(255,170,51,0.22);
  --color-error-border:   rgba(255,74,18,0.22);
  --color-info-border:    rgba(122,170,255,0.22);

  /* Syntax */
  --color-syntax-kw:      #ff7a30;
  --color-syntax-fn:      #ffd166;
  --color-syntax-str:     #3ddc84;
  --color-syntax-type:    #7a9fff;
  --color-syntax-num:     #ff9966;
  --color-syntax-comment: #3a4558;

  /* Spacing */
  --sp-1:  4px;
  --sp-2:  8px;
  --sp-3:  12px;
  --sp-4:  16px;
  --sp-5:  20px;
  --sp-6:  24px;
  --sp-8:  32px;
  --sp-10: 40px;
  --sp-12: 48px;

  /* Radii */
  --r-sm: 3px;
  --r-md: 5px;
  --r-lg: 8px;

  /* Transitions */
  --ease: 0.15s ease;

  /* Typography */
  --font-display: 'Barlow Condensed', sans-serif;
  --font-body:    'Barlow', sans-serif;
  --font-mono:    'Fira Code', monospace;

  /* Provider accent colors */
  --color-provider-anthropic: #ff4a12;
  --color-provider-openai:    #ffaa33;
  --color-provider-local:     #7aaaff;
  --color-provider-custom:    #8a9aac;
}
```

---

*DESIGN.md — Forge IDE v0.1. Update this document whenever a design decision is made that establishes a new pattern. The document should always reflect the current state of the system, not the aspirational state.*
