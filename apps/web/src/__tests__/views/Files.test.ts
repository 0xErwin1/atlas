import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, DELETE, saveDownload, push } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
  saveDownload: vi.fn(),
  push: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST: vi.fn(), PATCH, DELETE },
}));
vi.mock('@/lib/download', () => ({ saveDownload }));
vi.mock('vue-router', () => ({
  useRouter: () => ({ push }),
  useRoute: () => ({ name: 'files', params: {} }),
}));

import { useWorkspaceStore } from '@/stores/workspace';
import Files from '@/views/Files.vue';

const noteFile = {
  id: 'a-note',
  file_name: 'policy.pdf',
  content_type: 'application/pdf',
  size_bytes: 2048,
  sha256: 'deadbeef',
  actor: { id: 'u1', type: 'user', display_name: 'Ana Perez' },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  content_url: '/api/workspaces/acme/attachments/a-note',
  owner: { kind: 'document', title: 'Runbook', document_slug: 'runbook' },
};

const taskFile = {
  ...noteFile,
  id: 'a-task',
  file_name: 'screenshot.png',
  content_type: 'image/png',
  owner: { kind: 'task', title: 'Fix login', task_readable_id: 'ATL-42' },
};

function page(items: unknown[]) {
  return { data: { items, has_more: false } };
}

async function mountFiles() {
  const wrapper = mount(Files, {
    attachTo: document.body,
    global: { stubs: { Teleport: true } },
  });
  await flushPromises();
  return wrapper;
}

describe('Files view', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    saveDownload.mockResolvedValue(true);
    useWorkspaceStore().activeWorkspaceSlug = 'acme';
    GET.mockResolvedValue(page([noteFile, taskFile]));
  });

  it('lists every file with its owner, uploader, size and upload date', async () => {
    const wrapper = await mountFiles();

    const rows = wrapper.findAll('[data-attachment-id]');
    expect(rows).toHaveLength(2);

    const noteRow = wrapper.get('[data-attachment-id="a-note"]');
    expect(noteRow.text()).toContain('policy.pdf');
    expect(noteRow.text()).toContain('Runbook');
    expect(noteRow.text()).toContain('Ana Perez');
    expect(noteRow.text()).toContain('2.0 KB');

    expect(wrapper.get('[data-attachment-id="a-task"]').text()).toContain('ATL-42');
  });

  it('opens the note or the task the file hangs off', async () => {
    const wrapper = await mountFiles();

    await wrapper.get('[data-attachment-id="a-note"] .atl-files-owner').trigger('click');
    expect(push).toHaveBeenCalledWith({ name: 'notes', params: { slug: 'runbook' } });

    await wrapper.get('[data-attachment-id="a-task"] .atl-files-owner').trigger('click');
    expect(push).toHaveBeenCalledWith({ name: 'task-detail', params: { readableId: 'ATL-42' } });
  });

  it('downloads through the API client so the desktop bridge carries the bytes', async () => {
    const wrapper = await mountFiles();
    const blob = { size: 3 } as Blob;
    GET.mockResolvedValue({ data: blob });

    await wrapper.get('[data-attachment-id="a-note"] [title="Download"]').trigger('click');
    await flushPromises();

    expect(GET).toHaveBeenLastCalledWith(
      '/api/workspaces/{ws}/attachments/{attachment_id}',
      expect.objectContaining({
        params: { path: { ws: 'acme', attachment_id: 'a-note' } },
        parseAs: 'blob',
      }),
    );
    expect(saveDownload).toHaveBeenCalledWith(blob, 'policy.pdf');
  });

  it('renames through the workspace route and shows the new name in place', async () => {
    const wrapper = await mountFiles();
    PATCH.mockResolvedValue({ data: { ...noteFile, file_name: 'renamed.pdf' } });

    await wrapper.get('[data-attachment-id="a-note"] [title="Rename"]').trigger('click');
    await flushPromises();

    await wrapper.get('[role="dialog"] input').setValue('renamed.pdf');
    const confirm = wrapper.findAll('button').find((button) => button.text() === 'Rename');
    await confirm?.trigger('click');
    await flushPromises();

    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/attachments/{attachment_id}', {
      params: { path: { ws: 'acme', attachment_id: 'a-note' } },
      body: { file_name: 'renamed.pdf' },
    });
    expect(wrapper.get('[data-attachment-id="a-note"]').text()).toContain('renamed.pdf');
  });

  it('deletes only after the confirmation is accepted', async () => {
    const wrapper = await mountFiles();
    DELETE.mockResolvedValue({});

    await wrapper.get('[data-attachment-id="a-note"] [title="Delete"]').trigger('click');
    await flushPromises();
    expect(DELETE).not.toHaveBeenCalled();

    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/attachments/{attachment_id}', {
      params: { path: { ws: 'acme', attachment_id: 'a-note' } },
    });
    expect(wrapper.find('[data-attachment-id="a-note"]').exists()).toBe(false);
  });

  it('shows an empty state instead of a bare table when nothing matches', async () => {
    GET.mockResolvedValue(page([]));
    const wrapper = await mountFiles();

    expect(wrapper.find('[data-attachment-id]').exists()).toBe(false);
    expect(wrapper.text()).toContain('No files match these filters.');
  });
});
