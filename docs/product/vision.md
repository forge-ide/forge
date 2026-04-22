# Vision

> Extracted from CONCEPT.md — what Forge is, the mental model shift, and what success looks like

---

## 1. What Forge is

Forge is a **native desktop workshop for agentic work**. The session is the unit of work; the dashboard is home; the editor is one of several peer pane types, not the center of gravity.

The one-sentence pitch: **Any AI. One editor. Transparent by default.**

### The shift the concept makes

The original PDF described Forge in IDE terms (activity bar, file explorer, git), with the AI dashboard as the primary window. The refined concept inverts the mental model:

- The **session** is the unit of work, not the workspace.
- The **dashboard** is the home screen, not the editor.
- The **editor** is one of several peer pane types that live inside a session — chat, terminal, editor, files, and agent monitor — not the main event. See `architecture/session-layout.md §4.1` for the full list.

This matters because it determines everything downstream: window hierarchy, the CLI surface, sandboxing boundaries, config file locations, and what we even call the top-level concept.

### What Forge is not

- Not a chat app with a file tree.
- Not locked to any model provider, storage backend, or protocol.
- Not a replacement for `vim`/`code`/`zed` for heads-down coding with no AI.

---

## 14. What success looks like

Forge v1 ships when a thoughtful developer can adopt it without a tutorial — when the first session feels obvious, when every AI action is visible, when detach-and-reattach is a fluid motion not a recovery operation, and when running the same agent from the CLI feels as natural as opening the GUI. Success is not a checklist; it's the feeling that the tool is on your side.
