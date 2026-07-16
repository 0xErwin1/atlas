import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';

describe('PromptDialog', () => {
  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is named by its visible title', async () => {
    const wrapper = mount(PromptDialog, {
      props: { open: true, title: 'Rename attachment' },
      attachTo: document.body,
      global: { stubs: { Teleport: true } },
    });
    await nextTick();

    const dialog = wrapper.get('[role="dialog"]');
    const titleId = dialog.attributes('aria-labelledby');
    expect(titleId).toBeTruthy();
    expect(document.getElementById(titleId ?? '')?.textContent).toContain('Rename attachment');

    wrapper.unmount();
  });

  it('traps focus while open and restores the invoking control when closed', async () => {
    const invoker = document.createElement('button');
    invoker.textContent = 'Rename';
    document.body.append(invoker);
    invoker.focus();

    const wrapper = mount(PromptDialog, {
      props: { open: false, title: 'Rename attachment' },
      attachTo: document.body,
      global: { stubs: { Teleport: true } },
    });
    await wrapper.setProps({ open: true });
    await nextTick();

    const input = wrapper.get('input').element;
    const buttons = wrapper.findAll('button');
    const lastButton = buttons.at(-1)?.element;
    expect(document.activeElement).toBe(input);

    lastButton?.focus();
    await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(input);

    input.focus();
    await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(lastButton);

    await wrapper.setProps({ open: false });
    await nextTick();
    expect(document.activeElement).toBe(invoker);

    wrapper.unmount();
  });
});
