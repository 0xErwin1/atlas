import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, saveDownload } = vi.hoisted(() => ({ GET: vi.fn(), saveDownload: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST: vi.fn(), PATCH: vi.fn(), DELETE: vi.fn() },
}));
vi.mock('@/lib/download', () => ({ saveDownload }));

import AttachmentList from '@/components/tareas/AttachmentList.vue';
import { useUiStore } from '@/stores/ui';

const attachment = {
  id: 'att-1',
  file_name: 'report.pdf',
  content_type: 'application/pdf',
  size_bytes: 10,
  created_by: { id: 'u1', type: 'user' as const, display_name: 'Ann' },
  created_at: '2026-01-01T00:00:00Z',
};

function list() {
  return mount(AttachmentList, {
    props: { attachments: [attachment], ws: 'acme', readableId: 'ATL-1' },
  });
}

describe('AttachmentList downloads', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    GET.mockReset();
    saveDownload.mockReset().mockResolvedValue(true);
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();
  });

  it('fetches the bytes through the API client so the desktop bridge carries them', async () => {
    const blob = { size: 3 } as Blob;
    GET.mockResolvedValue({ data: blob });

    const wrapper = list();
    await wrapper.get('[aria-label="Download report.pdf"]').trigger('click');
    await flushPromises();

    expect(GET).toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content',
      expect.objectContaining({
        params: { path: { ws: 'acme', readable_id: 'ATL-1', attachment_id: 'att-1' } },
        parseAs: 'blob',
      }),
    );
    expect(saveDownload).toHaveBeenCalledWith(blob, 'report.pdf');
  });

  it('reports a failed download instead of failing silently', async () => {
    GET.mockResolvedValue({ error: { status: 404 } });
    const ui = useUiStore();
    const showBanner = vi.spyOn(ui, 'showBanner');

    const wrapper = list();
    await wrapper.get('[aria-label="Download report.pdf"]').trigger('click');
    await flushPromises();

    expect(saveDownload).not.toHaveBeenCalled();
    expect(showBanner).toHaveBeenCalledWith(expect.stringContaining('download'), 'error');
  });
});
