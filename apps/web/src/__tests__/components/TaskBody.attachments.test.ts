import { flushPromises, mount, shallowMount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { PATCH } = vi.hoisted(() => ({ PATCH: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn(), POST: vi.fn(), PATCH, DELETE: vi.fn() },
}));

import AttachmentList from '@/components/tareas/AttachmentList.vue';
import TaskBody from '@/components/tareas/TaskBody.vue';
import { useTagsStore } from '@/stores/tags';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';

const task = {
  id: 'task-1',
  readable_id: 'ATL-1',
  board_id: 'board-1',
  board_name: 'Board',
  column_id: 'column-1',
  column_name: 'Todo',
  title: 'Task',
  description: '',
  project_id: 'project-1',
  workspace_id: 'workspace-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
  created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
};

describe('TaskBody attachment picker', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    PATCH.mockReset();
  });

  it('keeps the attachments heading visible when the ready collection is empty', () => {
    const detail = useTaskDetailStore();
    const tags = useTagsStore();
    vi.spyOn(tags, 'load').mockResolvedValue();
    detail._setForTest({ attachments: [] });
    detail.collectionStatus = { ...detail.collectionStatus, attachments: 'ready' };
    detail.collectionLoaded = { ...detail.collectionLoaded, attachments: true };

    const wrapper = shallowMount(TaskBody, { props: { task, ws: 'ws' } });

    expect(wrapper.findAll('.atl-tv-section-label').map((label) => label.text())).toContain('Attachments');
  });

  it('selects and uploads multiple files sequentially while preserving batch feedback', async () => {
    const detail = useTaskDetailStore();
    const tags = useTagsStore();
    const ui = useUiStore();
    const uploadOrder: string[] = [];
    let activeUploads = 0;
    let maxActiveUploads = 0;

    const uploadAttachment = vi
      .spyOn(detail, 'uploadAttachment')
      .mockImplementation(async (_ws, _taskId, file) => {
        uploadOrder.push(file.name);
        activeUploads += 1;
        maxActiveUploads = Math.max(maxActiveUploads, activeUploads);
        await Promise.resolve();
        activeUploads -= 1;

        if (file.name === 'broken.txt') {
          detail.error = 'Upload failed';
          return false;
        }

        return true;
      });
    vi.spyOn(tags, 'load').mockResolvedValue();
    const showBanner = vi.spyOn(ui, 'showBanner');
    const wrapper = shallowMount(TaskBody, { props: { task, ws: 'ws' } });
    const input = wrapper.get('input[type="file"]');
    const files = [
      new File(['first'], 'first.txt'),
      new File(['broken'], 'broken.txt'),
      new File(['last'], 'last.txt'),
    ];

    expect(input.attributes('multiple')).toBeDefined();

    Object.defineProperty(input.element, 'files', { configurable: true, value: files });
    await input.trigger('change');
    await flushPromises();

    expect(uploadOrder).toEqual(['first.txt', 'broken.txt', 'last.txt']);
    expect(maxActiveUploads).toBe(1);
    expect(input.element).toHaveProperty('value', '');
    expect(showBanner).toHaveBeenCalledWith('Upload failed', 'error');
    expect(showBanner).toHaveBeenCalledWith('2 attachments uploaded', 'success');
    expect(wrapper.get('.atl-tv-attach').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('.atl-tv-attach').text()).toContain('Attach file');

    Object.defineProperty(input.element, 'files', { configurable: true, value: [files[0]] });
    await input.trigger('change');
    await flushPromises();

    expect(uploadAttachment).toHaveBeenCalledTimes(4);
    expect(uploadAttachment).toHaveBeenLastCalledWith('ws', 'ATL-1', files[0]);
  });

  it('renames an attachment through the PATCH API and updates the visible name', async () => {
    const detail = useTaskDetailStore();
    const attachment = {
      id: 'attachment-1',
      file_name: 'current name.txt',
      content_type: 'text/plain',
      size_bytes: 12,
      created_at: '2026-01-01T00:00:00Z',
      created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
    };
    detail._setForTest({ attachments: [attachment] });
    PATCH.mockResolvedValueOnce({
      data: { ...attachment, file_name: 'renamed.txt' },
      error: undefined,
    });

    const wrapper = mount(AttachmentList, {
      props: { attachments: detail.attachments, ws: 'ws', readableId: 'ATL-1' },
      global: { stubs: { Icon: true, Teleport: true } },
    });

    await wrapper.get('button[aria-label="Rename attachment current name.txt"]').trigger('click');
    const input = wrapper.get('input');
    expect(input.element).toHaveProperty('value', 'current name.txt');

    await input.setValue('renamed.txt');
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(PATCH).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}',
      {
        params: {
          path: { ws: 'ws', readable_id: 'ATL-1', attachment_id: 'attachment-1' },
        },
        body: { file_name: 'renamed.txt' },
      },
    );
    expect(wrapper.get('.atl-att-name').text()).toBe('renamed.txt');
    expect(wrapper.get('.atl-att-name').attributes()).toMatchObject({
      download: 'renamed.txt',
      href: '/api/workspaces/ws/tasks/ATL-1/attachments/attachment-1/content',
    });

    await wrapper.get('button[aria-label="Remove attachment renamed.txt"]').trigger('click');
    expect(wrapper.emitted('remove')).toEqual([['attachment-1']]);
  });

  it('does not let an earlier rename response close a newer rename dialog', async () => {
    const detail = useTaskDetailStore();
    const attachments = [
      {
        id: 'attachment-1',
        file_name: 'first.txt',
        content_type: 'text/plain',
        size_bytes: 12,
        created_at: '2026-01-01T00:00:00Z',
        created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
      },
      {
        id: 'attachment-2',
        file_name: 'second.txt',
        content_type: 'text/plain',
        size_bytes: 24,
        created_at: '2026-01-02T00:00:00Z',
        created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
      },
    ];
    detail._setForTest({ attachments });
    let resolveFirstRename: ((response: unknown) => void) | undefined;
    PATCH.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFirstRename = resolve;
      }),
    );

    const wrapper = mount(AttachmentList, {
      props: { attachments: detail.attachments, ws: 'ws', readableId: 'ATL-1' },
      global: { stubs: { Icon: true, Teleport: true } },
    });

    await wrapper.get('button[aria-label="Rename attachment first.txt"]').trigger('click');
    await wrapper.get('input').setValue('first-renamed.txt');
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await wrapper.get('[role="dialog"] button:first-child').trigger('click');
    await wrapper.get('button[aria-label="Rename attachment second.txt"]').trigger('click');
    await wrapper.get('input').setValue('second draft.txt');

    resolveFirstRename?.({
      data: { ...attachments[0], file_name: 'first-renamed.txt' },
      error: undefined,
    });
    await flushPromises();

    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.get('input').element).toHaveProperty('value', 'second draft.txt');
    expect(wrapper.get('.atl-att-name').text()).toBe('first-renamed.txt');
  });

  it('keeps the rename prompt open and surfaces the API hint when rename fails', async () => {
    const detail = useTaskDetailStore();
    const ui = useUiStore();
    detail._setForTest({
      attachments: [
        {
          id: 'attachment-1',
          file_name: 'current.txt',
          content_type: 'text/plain',
          size_bytes: 12,
          created_at: '2026-01-01T00:00:00Z',
          created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
        },
      ],
    });
    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Rename is not allowed' } });
    const showBanner = vi.spyOn(ui, 'showBanner');
    const wrapper = mount(AttachmentList, {
      props: { attachments: detail.attachments, ws: 'ws', readableId: 'ATL-1' },
      global: { stubs: { Icon: true, Teleport: true } },
    });

    await wrapper.get('button[aria-label="Rename attachment current.txt"]').trigger('click');
    await wrapper.get('input').setValue('blocked.txt');
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(showBanner).toHaveBeenCalledWith('Rename is not allowed', 'error');
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.get('.atl-att-name').text()).toBe('current.txt');
  });

  it('recovers from a rejected rename request and permits a retry', async () => {
    const detail = useTaskDetailStore();
    const ui = useUiStore();
    const attachment = {
      id: 'attachment-1',
      file_name: 'current.txt',
      content_type: 'text/plain',
      size_bytes: 12,
      created_at: '2026-01-01T00:00:00Z',
      created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
    };
    detail._setForTest({ attachments: [attachment] });
    PATCH.mockRejectedValueOnce(new TypeError('Failed to fetch')).mockResolvedValueOnce({
      data: { ...attachment, file_name: 'retry.txt' },
      error: undefined,
    });
    const showBanner = vi.spyOn(ui, 'showBanner');
    const wrapper = mount(AttachmentList, {
      props: { attachments: detail.attachments, ws: 'ws', readableId: 'ATL-1' },
      attachTo: document.body,
      global: { stubs: { Icon: true, Teleport: true } },
    });

    await wrapper.get('button[aria-label="Rename attachment current.txt"]').trigger('click');
    await wrapper.get('input').setValue('retry.txt');
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(showBanner).toHaveBeenCalledWith('Failed to rename attachment', 'error');
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(PATCH).toHaveBeenCalledTimes(2);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.get('.atl-att-name').text()).toBe('retry.txt');
    wrapper.unmount();
  });

  it.each([
    ['   ', 'file_name must not be blank'],
    [`${'é'.repeat(100)}x`, 'file_name must be at most 200 bytes'],
  ])('shows validation for an invalid attachment name without calling the API', async (name, message) => {
    const detail = useTaskDetailStore();
    detail._setForTest({
      attachments: [
        {
          id: 'attachment-1',
          file_name: 'current.txt',
          content_type: 'text/plain',
          size_bytes: 12,
          created_at: '2026-01-01T00:00:00Z',
          created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
        },
      ],
    });
    PATCH.mockResolvedValueOnce({
      data: { ...detail.attachments[0], file_name: 'corrected.txt' },
      error: undefined,
    });
    const wrapper = mount(AttachmentList, {
      props: { attachments: detail.attachments, ws: 'ws', readableId: 'ATL-1' },
      global: { stubs: { Icon: true, Teleport: true } },
    });

    await wrapper.get('button[aria-label="Rename attachment current.txt"]').trigger('click');
    await wrapper.get('input').setValue(name);
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(PATCH).not.toHaveBeenCalled();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.get('[role="alert"]').text()).toBe(message);
    expect(wrapper.get('input').attributes('aria-invalid')).toBe('true');

    await wrapper.get('input').setValue('corrected.txt');
    await wrapper.get('[role="dialog"] button:last-child').trigger('click');
    await flushPromises();

    expect(PATCH).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.get('.atl-att-name').text()).toBe('corrected.txt');
  });

  it('retries detail collections with the authoritative task UUID', async () => {
    const detail = useTaskDetailStore();
    const tags = useTagsStore();
    const loadAll = vi.spyOn(detail, 'loadAll').mockResolvedValue();
    vi.spyOn(tags, 'load').mockResolvedValue();
    detail.collectionStatus = { ...detail.collectionStatus, assignees: 'error' };
    detail.collectionErrors = { ...detail.collectionErrors, assignees: 'Assignees unavailable' };

    const wrapper = shallowMount(TaskBody, {
      props: { task, ws: 'ws' },
      global: {
        stubs: {
          ErrorState: {
            emits: ['retry'],
            template: '<button type="button" @click="$emit(\'retry\')">Retry</button>',
          },
        },
      },
    });

    await wrapper.get('button').trigger('click');

    expect(loadAll).toHaveBeenCalledWith('ws', 'ATL-1', undefined, 'task-1');
  });
});
