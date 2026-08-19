import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Btn, { type BtnVariant } from '@/components/ui/Btn.vue';

const VARIANTS: BtnVariant[] = ['primary', 'secondary', 'ghost', 'danger'];

function styleOf(variant: BtnVariant): string {
  return mount(Btn, { props: { variant } }).get('button').attributes('style') ?? '';
}

describe('Btn', () => {
  it.each(VARIANTS)('%s sits on the shared button height with square corners', (variant) => {
    const style = styleOf(variant);

    expect(style).toContain('height: var(--h-button)');
    expect(style).not.toContain('border-radius');
  });

  // A destructive control has to be distinguishable without relying on hue: the
  // design gives it a doubled border so it still reads as destructive in
  // greyscale and on an e-ink panel.
  it('marks the destructive variant with a doubled border, not just a colour', () => {
    const style = styleOf('danger');

    expect(style).toContain('border: 2px solid var(--c-danger)');
    expect(style).toContain('color: var(--c-danger)');
    expect(style).toContain('background-color: transparent');
  });

  it('draws label text in the UI family, not the data family', () => {
    expect(styleOf('secondary')).toContain('font-family: var(--font-ui)');
  });

  it('emits click when enabled and stays silent when disabled', async () => {
    const enabled = mount(Btn, { props: { variant: 'primary' } });
    await enabled.get('button').trigger('click');
    expect(enabled.emitted('click')).toHaveLength(1);

    const disabled = mount(Btn, { props: { variant: 'primary', disabled: true } });
    await disabled.get('button').trigger('click');
    expect(disabled.emitted('click')).toBeUndefined();
  });
});
