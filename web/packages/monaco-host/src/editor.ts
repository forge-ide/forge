// F-121: Monaco editor instantiation + adapter onto `EditorLike`.
//
// This module imports `monaco-editor` and therefore must not be imported
// by anything reachable from the test harness — see README "Test harness".

import * as monaco from 'monaco-editor';
import type { EditorLike } from './protocol.js';
import { FORGE_THEME, FORGE_THEME_ID } from './theme.js';

/** Attach a Monaco instance to `host` and return it wrapped as `EditorLike`. */
export function mountEditor(host: HTMLElement): EditorLike {
  monaco.editor.defineTheme(FORGE_THEME_ID, FORGE_THEME);

  const instance = monaco.editor.create(host, {
    value: '',
    language: 'plaintext',
    theme: FORGE_THEME_ID,
    automaticLayout: true,
    minimap: { enabled: false },
    fontFamily: 'Fira Code, monospace',
    fontSize: 13,
    renderWhitespace: 'selection',
    scrollBeyondLastLine: false,
  });

  return {
    setValue(value) {
      instance.setValue(value);
    },
    getValue() {
      return instance.getValue();
    },
    focus() {
      instance.focus();
    },
    onDidChangeContent(cb) {
      const sub = instance.onDidChangeModelContent(() => cb(instance.getValue()));
      return { dispose: () => sub.dispose() };
    },
    dispose() {
      instance.dispose();
    },
  };
}
