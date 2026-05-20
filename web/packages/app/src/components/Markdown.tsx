// Markdown renderer for assistant messages.
//
// Providers emit CommonMark-ish text (lists, code blocks, headings, inline
// emphasis, links). Until we wired this up the chat surface rendered raw
// strings inside a `<p>`, so backticks read as literal characters and code
// blocks collapsed onto a single line.
//
// Pipeline: `marked.parse(text)` → CommonMark + GFM HTML, then
// `DOMPurify.sanitize(html)` to strip anything that could ride inline HTML
// from a compromised provider into the webview (script tags, on-*
// attributes, javascript: URLs). DOMPurify needs a DOM — jsdom in tests,
// the real webview at runtime.
//
// Streaming-friendly: the component is a thin reactive wrapper; the
// memoised `html()` accessor re-runs whenever `props.text` changes, so a
// streaming assistant turn re-renders cleanly chunk-by-chunk without any
// extra plumbing. Marked is synchronous and well under a millisecond on
// the message sizes we care about, so re-parsing on every chunk is fine.

import { type Component, createMemo } from 'solid-js';
import DOMPurify from 'dompurify';
import { marked, type MarkedOptions } from 'marked';

const MARKED_OPTIONS: MarkedOptions = {
  // GFM tables / strikethrough / autolinks — the conventions providers
  // actually use in practice. CommonMark hard-breaks (\n\n) still hold.
  gfm: true,
  // Honor single newlines as `<br>` inside paragraphs. Provider output
  // often relies on this for poetry / lists-within-paragraphs / etc.
  breaks: true,
};

export interface MarkdownProps {
  /** The raw markdown source. Re-parsed reactively when this changes. */
  text: string;
  /** Optional class forwarded to the wrapper element. */
  class?: string;
  /** Optional test-id forwarded to the wrapper element. */
  testid?: string;
}

/**
 * Render `text` as sanitized markdown.
 *
 * The output is wrapped in a `<div>` rather than a `<p>` because marked
 * emits block-level elements (`<p>`, `<pre>`, `<h1>`, `<ul>`, ...) and
 * nesting block elements inside `<p>` is invalid HTML — the browser
 * silently rewrites the DOM and the visual result is unpredictable.
 */
export const Markdown: Component<MarkdownProps> = (props) => {
  const html = createMemo<string>(() => {
    const source = props.text ?? '';
    if (source.length === 0) return '';
    const raw = marked.parse(source, MARKED_OPTIONS);
    // marked's typings declare a sync overload (`async: false` default in
    // v18); the runtime return value is a string. Force the narrow.
    const html = typeof raw === 'string' ? raw : '';
    return DOMPurify.sanitize(html);
  });

  return (
    <div
      class={props.class}
      data-testid={props.testid}
      // SolidJS dedicated prop — sets innerHTML reactively when html()
      // changes. Sanitization above is the trust boundary.
      innerHTML={html()}
    />
  );
};
