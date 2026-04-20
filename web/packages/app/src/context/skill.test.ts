import { describe, expect, it } from 'vitest';
import { createSkillResolver } from './skill';

describe('createSkillResolver.list', () => {
  it('filters by name and description substring', async () => {
    const resolver = createSkillResolver({
      listSkills: () => [
        { name: 'typescript-review', description: 'TS lint pass' },
        { name: 'rust-audit', description: 'Check cargo-deny' },
      ],
    });
    expect((await resolver.list('typescript')).map((c) => c.value)).toEqual([
      'typescript-review',
    ]);
    expect((await resolver.list('cargo')).map((c) => c.value)).toEqual([
      'rust-audit',
    ]);
  });

  it('labels skills with their description when present', async () => {
    const resolver = createSkillResolver({
      listSkills: () => [
        { name: 'typescript-review', description: 'TS lint pass' },
      ],
    });
    const [candidate] = await resolver.list('');
    expect(candidate!.label).toBe('typescript-review — TS lint pass');
  });
});

describe('createSkillResolver.resolve', () => {
  it('returns a pointer block, not the skill body', async () => {
    const resolver = createSkillResolver({
      listSkills: () => [],
    });
    const block = await resolver.resolve('typescript-review');
    expect(block).toEqual({
      type: 'skill',
      content: 'skill:typescript-review',
      meta: { skillName: 'typescript-review', pointer: true },
    });
  });
});
