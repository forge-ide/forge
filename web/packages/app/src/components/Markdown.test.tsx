import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { Markdown } from './Markdown';

afterEach(() => {
  cleanup();
});

describe('Markdown', () => {
  it('renders plain text inside a paragraph', () => {
    const { container } = render(() => <Markdown text="hello world" />);
    const p = container.querySelector('p');
    expect(p).not.toBeNull();
    expect(p?.textContent).toBe('hello world');
  });

  it('renders bold inline emphasis', () => {
    const { container } = render(() => <Markdown text="say **hi** to me" />);
    const strong = container.querySelector('strong');
    expect(strong).not.toBeNull();
    expect(strong?.textContent).toBe('hi');
  });

  it('renders a fenced code block as <pre><code>', () => {
    const source = '```\nlet x = 1;\n```';
    const { container } = render(() => <Markdown text={source} />);
    const pre = container.querySelector('pre');
    expect(pre).not.toBeNull();
    const code = pre?.querySelector('code');
    expect(code).not.toBeNull();
    expect(code?.textContent).toContain('let x = 1;');
  });

  it('renders an unordered list', () => {
    const source = '- first\n- second\n- third';
    const { container } = render(() => <Markdown text={source} />);
    const items = container.querySelectorAll('li');
    expect(items.length).toBe(3);
    expect(items[0]?.textContent).toBe('first');
    expect(items[2]?.textContent).toBe('third');
  });

  it('renders headings at the right level', () => {
    const { container } = render(() => <Markdown text={'# Heading One\n\n## Heading Two'} />);
    const h1 = container.querySelector('h1');
    const h2 = container.querySelector('h2');
    expect(h1?.textContent).toBe('Heading One');
    expect(h2?.textContent).toBe('Heading Two');
  });

  it('strips <script> tags as part of DOMPurify sanitization', () => {
    // The trust boundary on assistant output — a compromised provider
    // could ride inline HTML into the bubble. DOMPurify drops <script>
    // outright and removes on-* attribute handlers.
    const source = 'before <script>alert(1)</script> after';
    const { container } = render(() => <Markdown text={source} />);
    expect(container.querySelector('script')).toBeNull();
    expect(container.textContent).toContain('before');
    expect(container.textContent).toContain('after');
  });

  it('strips javascript: hrefs from links', () => {
    const source = '[evil](javascript:alert(1))';
    const { container } = render(() => <Markdown text={source} />);
    const link = container.querySelector('a');
    // DOMPurify either drops the href or rewrites it; either way the
    // raw `javascript:` URL must not survive to the rendered HTML.
    expect(link?.getAttribute('href') ?? '').not.toContain('javascript:');
  });

  it('renders nothing for an empty string', () => {
    const { container } = render(() => <Markdown text="" />);
    // No <p>, no <div content> — just the empty wrapper div.
    expect(container.querySelector('p')).toBeNull();
    expect(container.textContent).toBe('');
  });

  it('re-renders when the text signal changes (streaming-friendly)', () => {
    const [text, setText] = createSignal('first');
    const { container } = render(() => <Markdown text={text()} />);
    expect(container.querySelector('p')?.textContent).toBe('first');
    setText('first second');
    expect(container.querySelector('p')?.textContent).toBe('first second');
  });

  it('forwards the optional class and testid to the wrapper element', () => {
    const { container } = render(() => (
      <Markdown text="hi" class="custom-class" testid="md-wrapper" />
    ));
    const wrapper = container.querySelector('[data-testid="md-wrapper"]');
    expect(wrapper).not.toBeNull();
    expect(wrapper?.className).toBe('custom-class');
  });
});
