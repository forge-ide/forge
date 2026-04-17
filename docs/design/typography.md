# Typography

> Extracted from DESIGN.md §4 — font families, type scale, per-family rules, and ligature usage

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
