import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import BacklinksPanel from '@/components/notas/BacklinksPanel.vue';

describe('BacklinksPanel', () => {
  it('renders a comment backlink through its authorized task parent without legacy source metadata', async () => {
    const wrapper = mount(BacklinksPanel, {
      props: {
        status: 'ready',
        error: null,
        backlinks: [
          {
            display_title: 'Comment link',
            source_document_id: 'legacy-document',
            source_slug: 'legacy-slug',
            source_title: 'Legacy title',
            comment_source: {
              type: 'comment',
              comment_id: 'comment-8',
              parent: { type: 'task', id: 'task-8', readable_id: 'ATL-8', title: 'Authorized task' },
            },
          },
        ],
      },
      global: { stubs: { Icon: true } },
    });

    const row = wrapper.get('[data-backlink-id="comment-8"]');
    expect(row.text()).toContain('Authorized task');
    expect(row.text()).not.toContain('Legacy title');

    await row.trigger('click');
    expect(wrapper.emitted('navigate-task')).toEqual([['ATL-8']]);
  });
});
