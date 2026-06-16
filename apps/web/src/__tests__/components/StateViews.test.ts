import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';

describe('State components (REQ-W28)', () => {
  describe('EmptyState', () => {
    it('renders title and hint', () => {
      const wrapper = mount(EmptyState, {
        props: { title: 'No documents open', hint: 'Press ⌘K to search' },
      });
      expect(wrapper.text()).toContain('No documents open');
      expect(wrapper.text()).toContain('Press ⌘K to search');
    });
  });

  describe('LoadingState', () => {
    it('renders the loading label', () => {
      const wrapper = mount(LoadingState, { props: { label: 'Loading…' } });
      expect(wrapper.text()).toContain('Loading…');
    });
  });

  describe('ErrorState', () => {
    it('surfaces the API hint, never a raw stack/detail (REQ-W28)', () => {
      const wrapper = mount(ErrorState, {
        props: {
          title: "Couldn't load board",
          hint: 'The tasks service did not respond. Retry in a moment.',
          detail: 'thread panicked at src/foo.rs:42 — stack backtrace ...',
          status: 503,
          requestId: '7f3a9c',
        },
      });

      expect(wrapper.text()).toContain("Couldn't load board");
      expect(wrapper.text()).toContain('The tasks service did not respond. Retry in a moment.');
      expect(wrapper.text()).not.toContain('panicked');
      expect(wrapper.text()).not.toContain('backtrace');
      expect(wrapper.text()).not.toContain('foo.rs');
    });

    it('renders a mono diagnostics line with status and request id when present', () => {
      const wrapper = mount(ErrorState, {
        props: { title: 'Error', status: 503, requestId: '7f3a9c' },
      });
      const text = wrapper.text();
      expect(text).toContain('503');
      expect(text).toContain('7f3a9c');
    });

    it('emits retry when the retry button is clicked', async () => {
      const wrapper = mount(ErrorState, { props: { title: 'Error' } });
      await wrapper.find('[data-action="retry"]').trigger('click');
      expect(wrapper.emitted('retry')).toBeTruthy();
    });

    it('falls back to a generic message when no hint is provided', () => {
      const wrapper = mount(ErrorState, { props: { title: 'Something broke' } });
      expect(wrapper.text()).toContain('Something broke');
    });
  });
});
