// F-157: CommandPalette — keyboard-invoked overlay that lists commands
// registered via `./registry`. Opens on Cmd/Ctrl+K (primary, matches
// `docs/architecture/window-hierarchy.md` §3.3) and Cmd/Ctrl+Shift+P
// (alternate). Fuzzy-filters the live command list as the user types;
// Enter runs the active row; Escape / backdrop click / a second shortcut
// press closes it.
//
// The palette owns no commands itself — it is purely a UI surface over the
// module registry. Components elsewhere register entries via
// `registerCommand({ id, title, run })`; the palette re-reads the list on
// every open so registrations made mid-session surface immediately.

import {
  createEffect,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type Component,
} from 'solid-js';
import { filterCommandsByQuery, type Command } from './registry';
import { useFocusTrap } from '../lib/useFocusTrap';
import './CommandPalette.css';

interface ShortcutMatch {
  /** True when the event matches either palette shortcut. */
  open: boolean;
}

function matchShortcut(e: KeyboardEvent): ShortcutMatch {
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return { open: false };
  const key = e.key.toLowerCase();
  // Cmd/Ctrl+K (primary).
  if (key === 'k' && !e.shiftKey && !e.altKey) return { open: true };
  // Cmd/Ctrl+Shift+P (alternate).
  if (key === 'p' && e.shiftKey && !e.altKey) return { open: true };
  return { open: false };
}

export const CommandPalette: Component = () => {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [items, setItems] = createSignal<Command[]>([]);
  const [activeIndex, setActiveIndex] = createSignal(0);

  let inputRef: HTMLInputElement | undefined;

  // Re-evaluate the item list whenever the query or open-state changes. We
  // re-read the registry on open so late registrations are reflected.
  createEffect(() => {
    if (!open()) return;
    const filtered = filterCommandsByQuery(query());
    setItems(filtered);
    setActiveIndex((i) => {
      if (filtered.length === 0) return 0;
      if (i >= filtered.length) return 0;
      return i;
    });
  });

  const openPalette = (): void => {
    setQuery('');
    setActiveIndex(0);
    setOpen(true);
    // Focus of the input is now driven by useFocusTrap on the dialog body's
    // mount — no queueMicrotask shim needed.
  };

  const closePalette = (): void => {
    setOpen(false);
  };

  const runActive = (): void => {
    const list = items();
    const cmd = list[activeIndex()];
    if (!cmd) return;
    // Close first so the `run` handler (which may navigate) runs against a
    // clean DOM.
    closePalette();
    cmd.run();
  };

  const handleGlobalKeyDown = (e: KeyboardEvent): void => {
    const shortcut = matchShortcut(e);
    if (shortcut.open) {
      e.preventDefault();
      if (open()) closePalette();
      else openPalette();
    }
  };

  onMount(() => {
    if (typeof window !== 'undefined') {
      // Capture phase: guarantees we see the shortcut before any input
      // handler (monaco, xterm, etc.) swallows it.
      window.addEventListener('keydown', handleGlobalKeyDown, true);
    }
  });

  onCleanup(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener('keydown', handleGlobalKeyDown, true);
    }
  });

  const handleInputKeyDown = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.preventDefault();
      closePalette();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      const list = items();
      if (list.length === 0) return;
      setActiveIndex((i) => (i + 1) % list.length);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      const list = items();
      if (list.length === 0) return;
      setActiveIndex((i) => (i - 1 + list.length) % list.length);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      runActive();
      return;
    }
  };

  const handleInput = (e: InputEvent): void => {
    const target = e.currentTarget as HTMLInputElement;
    setQuery(target.value);
  };

  const onItemClick = (index: number) => (e: MouseEvent): void => {
    e.preventDefault();
    setActiveIndex(index);
    runActive();
  };

  const onBackdropClick = (): void => {
    closePalette();
  };

  // F-402: the dialog body is a separate component so its mount/unmount
  // lifecycle drives useFocusTrap — focus trap activates on open, focus
  // restore runs on close.
  const Body: Component = () => {
    let dialogRef: HTMLDivElement | undefined;
    useFocusTrap(() => dialogRef, { initialFocus: () => inputRef });
    return (
      <div
        class="command-palette__backdrop"
        data-testid="command-palette-backdrop"
        onClick={onBackdropClick}
      >
        <div
          ref={dialogRef}
          class="command-palette"
          data-testid="command-palette"
          role="dialog"
          aria-modal="true"
          aria-label="Command palette"
          onClick={(e) => e.stopPropagation()}
        >
          <input
            ref={inputRef}
            class="command-palette__input"
            data-testid="command-palette-input"
            type="text"
            placeholder="Type a command…"
            autocomplete="off"
            value={query()}
            onInput={handleInput}
            onKeyDown={handleInputKeyDown}
            aria-label="Command palette input"
          />
          <ul class="command-palette__list" role="listbox">
            <For each={items()}>
              {(cmd, index) => (
                <li
                  class="command-palette__item"
                  classList={{
                    'command-palette__item--active': index() === activeIndex(),
                  }}
                  data-testid="command-palette-item"
                  role="option"
                  aria-selected={index() === activeIndex()}
                  onClick={onItemClick(index())}
                >
                  <span class="command-palette__item-title">{cmd.title}</span>
                </li>
              )}
            </For>
            <Show when={items().length === 0}>
              <li
                class="command-palette__empty"
                data-testid="command-palette-empty"
                role="option"
                aria-disabled="true"
              >
                No matching commands
              </li>
            </Show>
          </ul>
        </div>
      </div>
    );
  };

  return (
    <Show when={open()}>
      <Body />
    </Show>
  );
};
