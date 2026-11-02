import { EditorState, RangeSet, StateField, StateEffect } from "@codemirror/state";
import {
  EditorView,
  Decoration,
  GutterMarker,
  gutter,
  WidgetType,
  type DecorationSet,
} from "@codemirror/view";
import { visibleText } from "./text";
export type DiffHunk = {
  id: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: string[];
  turnId?: string;
};
export type DiffRow = { text: string; old?: number; new?: number; hunk: DiffHunk; header: boolean };
export function diffRows(hunks: DiffHunk[]): DiffRow[] {
  return hunks.flatMap((hunk) => {
    let old = hunk.oldStart,
      head = hunk.newStart;
    return [
      { text: `@@ -${old},${hunk.oldLines} +${head},${hunk.newLines} @@`, hunk, header: true },
      ...hunk.lines.map((text) => ({
        text,
        hunk,
        header: false,
        ...(text.startsWith(" ") || text.startsWith("-") ? { old: old++ } : {}),
        ...(text.startsWith(" ") || text.startsWith("+") ? { new: head++ } : {}),
      })),
    ];
  });
}
export type DiffOptions = {
  onSelect: (range: { startLine: number; endLine: number }) => void;
  onRevert: (hunkId: string) => void;
  onWhy: (turnId: string) => void;
  onCopy: (text: string) => void;
  disabled?: boolean;
  widgets?: { line: number; create: () => HTMLElement }[];
};
export function mountDiff(parent: HTMLElement, hunks: DiffHunk[], options: DiffOptions) {
  const rows = diffRows(hunks);
  let start: number | undefined,
    dragging = false;
  const end = () => {
    dragging = false;
  };
  window.addEventListener("pointerup", end);
  function select(line: number) {
    if (options.disabled) return;
    start = line;
    dragging = true;
    applySelection({ startLine: line, endLine: line });
    options.onSelect({ startLine: line, endLine: line });
  }
  class NumberMarker extends GutterMarker {
    constructor(
      readonly value: number | undefined,
      readonly head: boolean,
    ) {
      super();
    }
    override eq(other: GutterMarker): boolean {
      return (
        other instanceof NumberMarker && other.value === this.value && other.head === this.head
      );
    }
    override toDOM() {
      const el = document.createElement(this.head && this.value !== undefined ? "button" : "span");
      el.textContent = this.value === undefined ? "" : String(this.value);
      if (this.head && this.value !== undefined) {
        const line = this.value;
        el.dataset.line = String(line);
        el.tabIndex = -1; // Accessible range inputs live outside CodeMirror’s hidden gutter.
        el.setAttribute("aria-label", `Comment on line ${line}`);
        el.addEventListener("pointerdown", (e) => {
          e.preventDefault();
          select(line);
        });
        el.addEventListener("pointerenter", () => {
          if (dragging && start !== undefined) {
            const range = { startLine: Math.min(start, line), endLine: Math.max(start, line) };
            applySelection(range);
            options.onSelect(range);
          }
        });
        el.addEventListener("click", (e) => {
          if (e instanceof MouseEvent && e.detail === 0) select(line);
        });
      }
      return el;
    }
  }
  class HunkMenu extends WidgetType {
    constructor(readonly hunk: DiffHunk) {
      super();
    }
    override toDOM() {
      const bar = document.createElement("div");
      bar.className = "diff-hunk-menu";
      function button(label: string, fn: () => void, disabled = false) {
        const el = document.createElement("button");
        el.type = "button";
        el.textContent = label;
        el.setAttribute("aria-label", label);
        el.disabled = disabled;
        el.addEventListener("click", fn);
        bar.append(el);
      }
      button("Revert hunk", () => options.onRevert(this.hunk.id), options.disabled);
      button("Copy", () =>
        options.onCopy(
          visibleText(
            `@@ -${this.hunk.oldStart},${this.hunk.oldLines} +${this.hunk.newStart},${this.hunk.newLines} @@\n${this.hunk.lines.join("\n")}`,
          ),
        ),
      );
      if (this.hunk.turnId) {
        const turn = this.hunk.turnId;
        button("Show why", () => options.onWhy(turn));
        button("(?)", () => options.onWhy(turn));
      } else {
        const note = document.createElement("span");
        note.textContent = "Origin attribution unavailable";
        bar.append(note);
      }
      return bar;
    }
  }
  class CommentWidget extends WidgetType {
    constructor(readonly create: () => HTMLElement) {
      super();
    }
    override toDOM() {
      return this.create();
    }
    override ignoreEvent() {
      return false;
    }
  }
  const doc = EditorState.create({ doc: rows.map((r) => visibleText(r.text)).join("\n") }).doc;
  const makeDecorations = () => {
    const decorations: import("@codemirror/state").Range<Decoration>[] = [];
    rows.forEach((row, i) => {
      const line = doc.line(i + 1);
      decorations.push(
        Decoration.line({
          class: row.header
            ? "diff-header"
            : row.text.startsWith("+")
              ? "diff-added"
              : row.text.startsWith("-")
                ? "diff-removed"
                : "diff-context",
        }).range(line.from),
      );
      if (row.header)
        decorations.push(
          Decoration.widget({ widget: new HunkMenu(row.hunk), block: true, side: -1 }).range(
            line.from,
          ),
        );
      for (const widget of options.widgets ?? [])
        if (row.new === widget.line)
          decorations.push(
            Decoration.widget({
              widget: new CommentWidget(widget.create),
              block: true,
              side: 1,
            }).range(line.to),
          );
    });
    return Decoration.set(decorations, true);
  };
  type Range = { startLine: number; endLine: number };
  const selectionEffect = StateEffect.define<Range | undefined>();
  const selectionField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(value, transaction) {
      for (const effect of transaction.effects)
        if (effect.is(selectionEffect)) {
          const selected = effect.value;
          return Decoration.set(
            rows.flatMap((row, index) =>
              selected &&
              row.new !== undefined &&
              row.new >= selected.startLine &&
              row.new <= selected.endLine
                ? [Decoration.line({ class: "diff-selected" }).range(doc.line(index + 1).from)]
                : [],
            ),
          );
        }
      return value;
    },
    provide: (field) => EditorView.decorations.from(field),
  });
  function applySelection(range: Range | undefined) {
    view.dispatch({ effects: selectionEffect.of(range) });
  }
  const state = EditorState.create({
    doc,
    extensions: [
      selectionField,
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      EditorView.contentAttributes.of({ "aria-label": "Unified diff", tabindex: "0" }),
      EditorView.theme({
        "&": { height: "100%" },
        ".cm-scroller": { overflow: "auto", fontFamily: "ui-monospace, monospace" },
        ".cm-content": { padding: "0" },
        ".cm-line": { lineHeight: "22px" },
        ".cm-gutterElement": { lineHeight: "22px" },
        ".cm-gutters": { backgroundColor: "transparent", border: "none" },
      }),
      ...[false, true].map((head) => {
        const markers = RangeSet.of(
          rows.map((r, i) =>
            new NumberMarker(head ? r.new : r.old, head).range(doc.line(i + 1).from),
          ),
        );
        return gutter({
          class: head ? "diff-head-numbers" : "diff-old-numbers",
          markers: () => markers,
        });
      }),
      EditorView.decorations.of(makeDecorations()),
    ],
  });
  const view = new EditorView({ state, parent });
  return {
    view,
    setSelection: applySelection,
    destroy() {
      window.removeEventListener("pointerup", end);
      view.destroy();
    },
  };
}
