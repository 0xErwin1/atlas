import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';

const mountDialog = (props: Record<string, unknown> = {}) =>
  mount(ConfirmDialog, {
    props: { open: true, title: 'Delete task', ...props },
    global: { stubs: { teleport: true } },
  });

describe('ConfirmDialog', () => {
  it('renders the title and message', () => {
    const wrapper = mountDialog({ message: 'This cannot be undone.' });
    expect(wrapper.text()).toContain('Delete task');
    expect(wrapper.text()).toContain('This cannot be undone.');
  });

  it('emits confirm when the confirm button is clicked', async () => {
    const wrapper = mountDialog({ confirmLabel: 'Delete' });
    await wrapper.get('[data-test="confirm"]').trigger('click');
    expect(wrapper.emitted('confirm')).toHaveLength(1);
  });

  it('emits cancel from the cancel button, the close button, and Escape', async () => {
    const wrapper = mountDialog();

    await wrapper.get('[data-test="cancel"]').trigger('click');
    await wrapper.get('[data-test="close"]').trigger('click');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));

    expect(wrapper.emitted('cancel')).toHaveLength(3);
  });

  it('renders the affected-resource detail row only when detail is given', () => {
    const without = mountDialog();
    expect(without.find('[data-test="detail"]').exists()).toBe(false);

    const wrapper = mountDialog({ detail: 'mara@atlas.dev · Editor', detailIcon: 'user' });
    expect(wrapper.get('[data-test="detail"]').text()).toContain('mara@atlas.dev · Editor');
  });

  it('renders the secondary fallout note only when note is given', () => {
    const without = mountDialog();
    expect(without.find('[data-test="note"]').exists()).toBe(false);

    const wrapper = mountDialog({ note: 'Re-invite them at any time.' });
    expect(wrapper.get('[data-test="note"]').text()).toContain('Re-invite them at any time.');
  });

  it('uses the danger tone (from the legacy danger prop) on the confirm button', () => {
    const wrapper = mountDialog({ danger: true });
    expect(wrapper.get('[data-test="confirm"]').attributes('style')).toContain('var(--c-danger)');
  });

  it('uses a non-danger confirm button for the warning tone', () => {
    const wrapper = mountDialog({ tone: 'warning' });
    const style = wrapper.get('[data-test="confirm"]').attributes('style') ?? '';
    expect(style).not.toContain('var(--c-danger)');
  });
});
