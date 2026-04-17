# Spacing & Layout

> Extracted from DESIGN.md §5 — base-4 spacing scale, border radii, shell structure, and surface elevation order

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
