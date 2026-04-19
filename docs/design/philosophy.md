# Design Philosophy

> Extracted from DESIGN.md §1-2 — principles, visual identity, mark construction, wordmark, and colorways

**Related docs:** [Color System](color-system.md) · [Typography](typography.md) · [Voice & Terminology](voice-terminology.md)

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

The hex values below are referenced by name (`ember-400`, `iron-900`, etc.) elsewhere in the design system; see [Color System](color-system.md) for the full Ember and Iron scales and [Token Reference](token-reference.md) for the corresponding CSS custom properties.

| Context | Background | Usage |
|---|---|---|
| Dark (primary) | `#07080a` | All in-app UI |
| Light (alternate) | `#f0ede8` | Light mode |
| Ember (reversed) | `#ff4a12` | Status bar, CTAs on dark backgrounds |
| Monochrome | `#13161d` | Single-color contexts |
