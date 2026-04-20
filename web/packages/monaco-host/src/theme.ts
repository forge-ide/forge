// F-121: Forge design-token theme for Monaco.
//
// `monaco.editor.defineTheme()` takes literal color strings; it does not
// resolve CSS custom properties. The hex values below mirror
// `web/packages/design/src/tokens.css` (authoritative source
// `docs/design/token-reference.md`). `scripts/check-tokens.mjs` does NOT
// cover this file — if tokens change, update both in the same PR. See
// README "Theme drift" for the rationale.

/**
 * Token hex values, pinned from tokens.css. Keep this list small — only
 * the tokens the Monaco theme actually consumes.
 */
export const TOKENS = {
  bg: '#07080a',
  surface1: '#0d0f13',
  surface2: '#13161d',
  surface3: '#181c26',
  border1: '#1c2230',
  border2: '#252f3e',
  textPrimary: '#eae6de',
  textSecondary: '#8a9aac',
  textTertiary: '#3a4558',
  ember400: '#ff4a12',
  ember300: '#ff7a30',
  syntaxKw: '#ff7a30',
  syntaxFn: '#ffd166',
  syntaxStr: '#3ddc84',
  syntaxType: '#7a9fff',
  syntaxNum: '#ff9966',
  syntaxComment: '#3a4558',
  info: '#7aaaff',
  success: '#3ddc84',
  error: '#ff4a12',
} as const;

export const FORGE_THEME_ID = 'forge-ember';

/** Monaco `IStandaloneThemeData` shape (typed loosely to avoid an import). */
export interface ForgeThemeData {
  base: 'vs-dark';
  inherit: boolean;
  rules: Array<{ token: string; foreground?: string; fontStyle?: string }>;
  colors: Record<string, string>;
}

/** Theme definition in the shape Monaco's `defineTheme` accepts. */
export const FORGE_THEME: ForgeThemeData = {
  base: 'vs-dark',
  inherit: true,
  rules: [
    { token: 'comment', foreground: TOKENS.syntaxComment.slice(1), fontStyle: 'italic' },
    { token: 'keyword', foreground: TOKENS.syntaxKw.slice(1) },
    { token: 'string', foreground: TOKENS.syntaxStr.slice(1) },
    { token: 'number', foreground: TOKENS.syntaxNum.slice(1) },
    { token: 'type', foreground: TOKENS.syntaxType.slice(1) },
    { token: 'function', foreground: TOKENS.syntaxFn.slice(1) },
    { token: 'operator', foreground: TOKENS.ember300.slice(1) },
  ],
  colors: {
    'editor.background': TOKENS.bg,
    'editor.foreground': TOKENS.textPrimary,
    'editorCursor.foreground': TOKENS.ember400,
    'editor.lineHighlightBackground': TOKENS.surface2,
    'editor.selectionBackground': TOKENS.surface3,
    'editorLineNumber.foreground': TOKENS.textTertiary,
    'editorLineNumber.activeForeground': TOKENS.textSecondary,
    'editor.inactiveSelectionBackground': TOKENS.border1,
    'editorIndentGuide.background': TOKENS.border1,
    'editorIndentGuide.activeBackground': TOKENS.border2,
    'editorError.foreground': TOKENS.error,
    'editorWarning.foreground': TOKENS.ember300,
    'editorInfo.foreground': TOKENS.info,
  },
};
