# Voice & Terminology

> Extracted from DESIGN.md §8-9 — copy principles, formatting rules, terminology table, and do/don't reference

**Related docs:** [Design Philosophy](philosophy.md) · [Component Principles](component-principles.md) · [AI Patterns](ai-patterns.md)

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
- Empty-state copy: noun phrase describing what would be here, plus an optional verb-noun CTA. `No agents running` or `No agents running · start agent`, never `There are no agents currently running`. See [AI Patterns](ai-patterns.md) §"Interaction states" for the surrounding treatment.

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

The `ember-400` and `iron` color names referenced below are defined in the [Color System](color-system.md) and exposed as CSS custom properties in [Token Reference](token-reference.md).

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
