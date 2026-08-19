import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Chip, { type ChipTone } from '@/components/ui/Chip.vue';

const TONES: ChipTone[] = ['info', 'success', 'warning', 'danger', 'agent', 'neutral'];

function styleOf(tone: ChipTone): string {
  return (
    mount(Chip, { props: { tone }, slots: { default: 'mcp' } })
      .get('span')
      .attributes('style') ?? ''
  );
}

describe('Chip', () => {
  // Every tone used to carry the previous palette baked in as an rgba literal,
  // so a theme swap left the chips behind. Each one now has to resolve through
  // the tokens instead.
  it.each(TONES)('%s derives its colours from tokens', (tone) => {
    const style = styleOf(tone);

    expect(style).not.toMatch(/rgba\(/);
    expect(style).toContain('var(--c-');
  });

  it.each(TONES)('%s keeps the square blueprint corner', (tone) => {
    expect(styleOf(tone)).not.toMatch(/border-radius:\s*[^0]/);
  });

  it('renders its label as the chip content', () => {
    expect(mount(Chip, { slots: { default: 'mcp' } }).text()).toBe('mcp');
  });
});
