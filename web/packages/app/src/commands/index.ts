// Public surface for the F-157 command-palette module. Importing from
// `'./commands'` anywhere inside `packages/app` is the supported entry
// point. The palette component itself renders at the shell level in
// `App.tsx`; other consumers call `registerCommand` to expose actions.

export { CommandPalette } from './CommandPalette';
export {
  filterCommandsByQuery,
  fuzzyMatch,
  listCommands,
  registerCommand,
  unregisterCommand,
  type Command,
  type FuzzyMatchResult,
} from './registry';
export { registerBuiltins } from './registerBuiltins';
