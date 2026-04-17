# Token Pipeline

> Extracted from IMPLEMENTATION.md §9.4 — design token drift check and CI enforcement

---

## 9.4 Design token pipeline

- `web/packages/design/src/tokens.css` is the single source of CSS vars
- Checked against DESIGN.md by a CI script (`scripts/check-tokens.sh`) that parses both and fails on drift
- Component files never use raw hex — lint rule enforces `var(--color-*)`

### Usage in component code

```css
/* Correct — use tokens */
color: var(--color-text-primary);
background: var(--color-surface-2);
border-color: var(--color-border-brand);

/* Wrong — never raw hex in component code */
color: #eae6de;
background: #13161d;
```

### CI check

`scripts/check-tokens.sh` runs as a CI step. It:
1. Parses token definitions from `web/packages/design/src/tokens.css`
2. Parses the token reference table in `docs/design/color-system.md`
3. Fails the build if any token drifts between the two sources

The script runs after `cargo test` and before the frontend build. Fix drift by updating `tokens.css` to match the design doc (design doc is authoritative) or, if intentional, updating both simultaneously.
