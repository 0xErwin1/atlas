import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resourceCache } from '@/cache/cacheRuntime';
import TrashPanel from '@/components/settings/TrashPanel.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import type { TrashItem } from '@/stores/trash';
import { useTrashStore } from '@/stores/trash';
import { useWorkspaceStore } from '@/stores/workspace';

const item = {
  kind: 'project' as const,
  target_id: '018f4abc-1234-7abc-8def-0123456789ab',
  workspace_id: '018f4abc-1234-7abc-8def-0123456789ac',
  deleted_at: '2026-07-22T00:00:00Z',
};

function mountPanel() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [];
  vi.spyOn(workspace, 'loadAdminWorkspaces').mockResolvedValue();
  vi.spyOn(workspace, 'loadProjects').mockResolvedValue();

  const trash = useTrashStore();
  vi.spyOn(trash, 'load').mockResolvedValue();
  return { trash, wrapper: mount(TrashPanel) };
}

describe('TrashPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('requires the exact typed project purge confirmation before issuing a purge', async () => {
    const { trash, wrapper } = mountPanel();
    const purge = vi.spyOn(trash, 'purge').mockResolvedValue(null);
    const vm = wrapper.vm as unknown as { purgeTarget: typeof item | null };
    vm.purgeTarget = item;
    await wrapper.vm.$nextTick();

    const dialog = wrapper.findComponent(ConfirmDialog);
    expect(dialog.props('confirmationText')).toBe(`PURGE ${item.target_id}`);
    const input = document.body.querySelector('.atl-confirm-input') as HTMLInputElement;
    const confirm = document.body.querySelector('[data-test="confirm"]') as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);

    input.value = `PURGE ${item.target_id}`;
    input.dispatchEvent(new Event('input'));
    await flushPromises();
    expect(confirm.disabled).toBe(false);

    confirm.click();
    await flushPromises();
    expect(purge).toHaveBeenCalledWith(item);
    wrapper.unmount();
  });

  it('keeps a failed poll pending for automatic and manual retry', async () => {
    vi.useFakeTimers();
    const { trash, wrapper } = mountPanel();
    const pending = { ...item, operation_id: 'op', status: 'cleanup_pending' as const, attempts: 1 };
    const complete = { ...pending, status: 'complete' as const, attempts: 2 };
    vi.spyOn(trash, 'purge').mockResolvedValue(pending);
    const poll = vi.spyOn(trash, 'poll').mockResolvedValueOnce(null).mockResolvedValueOnce(complete);
    const vm = wrapper.vm as unknown as {
      purgeTarget: typeof item | null;
      confirmPurge: () => Promise<void>;
      operation: typeof pending | null;
      poll: () => Promise<void>;
    };
    vm.purgeTarget = item;
    await vm.confirmPurge();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(vm.operation?.status).toBe('cleanup_pending');

    await vm.poll();
    expect(poll).toHaveBeenCalledTimes(2);
    expect(vm.operation?.status).toBe('complete');
    wrapper.unmount();
    vi.useRealTimers();
  });

  it.each([
    ['project', 'project:atlas'],
    ['folder', 'folder:target-id'],
    ['document', 'document:target-id'],
    ['comment', 'comment:target-id'],
    ['attachment', 'attachment:target-id'],
  ] as const)('refreshes cache state after restoring a %s without navigation', async (kind, requiredTag) => {
    const { trash, wrapper } = mountPanel();
    const workspace = useWorkspaceStore();
    workspace.projects = [
      {
        id: 'project-id',
        slug: 'atlas',
        name: 'Atlas',
        task_prefix: 'ATL',
        workspace_id: 'workspace-id',
        visibility: 'workspace',
      },
    ];
    vi.spyOn(workspace, 'workspaceIdForSlug').mockReturnValue('workspace-id');
    vi.spyOn(trash, 'restore').mockResolvedValue(true);
    const purge = vi.spyOn(resourceCache, 'purgeTags').mockResolvedValue(true);
    const vm = wrapper.vm as unknown as { restore: (target: TrashItem) => Promise<void> };

    await vm.restore({ ...item, kind, target_id: kind === 'project' ? 'project-id' : 'target-id' });

    expect(workspace.loadProjects).toHaveBeenCalledWith('acme');
    expect(purge).toHaveBeenCalledWith(
      expect.arrayContaining(['workspace:workspace-id', requiredTag]),
      undefined,
      'workspace-id',
    );
    wrapper.unmount();
  });
});
