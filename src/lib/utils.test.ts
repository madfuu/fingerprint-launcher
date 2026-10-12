import { describe, expect, it } from 'vitest';

import { cn } from '@/lib/utils';

describe('cn', () => {
  it('joins conditional class values and drops falsy ones', () => {
    const state = { hidden: false, dense: true };
    expect(
      cn('px-2', state.hidden && 'hidden', null, undefined, {
        'text-xs': state.dense,
        'font-mono': !state.dense,
      })
    ).toBe('px-2 text-xs');
  });

  it('lets later Tailwind classes win conflicts', () => {
    expect(cn('px-2 py-1', 'px-4')).toBe('py-1 px-4');
    expect(cn('text-xs', 'text-[11px]')).toBe('text-[11px]');
    expect(cn('bg-primary', 'bg-destructive')).toBe('bg-destructive');
  });
});
