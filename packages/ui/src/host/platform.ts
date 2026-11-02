// Normal desktop build; only the explicit browser-test mode swaps this module.
export { createTauriTransport as createTransport } from "./daemon";
