import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SecretReveal from '@/components/ui/SecretReveal.vue';

describe('SecretReveal', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the warning, the value, and the optional caption slot', () => {
    const wrapper = mount(SecretReveal, {
      props: { value: 'sk_test_123', warning: 'Copy this now.' },
      slots: { caption: 'Secret for key "ci-bot"' },
    });

    expect(wrapper.text()).toContain('Copy this now.');
    expect(wrapper.get('[data-secret-value]').text()).toBe('sk_test_123');
    expect(wrapper.text()).toContain('Secret for key "ci-bot"');
  });

  it('copies the value and switches the button to the copied state', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    const wrapper = mount(SecretReveal, {
      props: { value: 'sk_test_123', warning: 'Copy this now.' },
    });

    await wrapper.get('[data-secret-copy]').trigger('click');
    await flushPromises();

    expect(writeText).toHaveBeenCalledWith('sk_test_123');
    expect(wrapper.get('[data-secret-copy]').text()).toContain('Copied');
  });
});
