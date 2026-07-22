import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { purgeTags } = vi.hoisted(() => ({ purgeTags: vi.fn() }));
const { currentRoute, push } = vi.hoisted(() => ({
  currentRoute: { value: { name: 'notes', params: { slug: 'doc-1' } } },
  push: vi.fn(),
}));

vi.mock('@/cache/cacheRuntime', () => ({
  getResourceCachePrincipal: () => 'user:principal',
  resourceCache: { purgeTags },
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ currentRoute, push }),
}));

import { useProjectDeletion } from '@/composables/useProjectDeletion';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useUiStore } from '@/stores/ui';
import { type ProjectSummary, useWorkspaceStore } from '@/stores/workspace';

const project: ProjectSummary = {
  id: 'project-id',
  slug: 'atlas',
  name: 'Atlas',
  task_prefix: 'ATL',
  workspace_id: 'workspace-id',
  visibility: 'workspace',
};

function seedProject(): ReturnType<typeof useWorkspaceStore> {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [project];
  vi.spyOn(workspace, 'workspaceIdForSlug').mockReturnValue('workspace-id');
  return workspace;
}

describe('useProjectDeletion', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    purgeTags.mockResolvedValue(true);
    currentRoute.value = { name: 'notes', params: { slug: 'doc-1' } };
  });

  it('refuses deletion while the project owns the dirty document tab', async () => {
    const workspace = seedProject();
    const remove = vi.spyOn(workspace, 'deleteProject').mockResolvedValue(true);
    const tabs = useNotesTabsStore();
    tabs.open('acme', { kind: 'doc', id: 'doc-1' }, 'Document', 'atlas');
    tabs.setDirtyDoc('acme', 'doc-1');

    const deleted = await useProjectDeletion().deleteProject(project);

    expect(deleted).toBe(false);
    expect(remove).not.toHaveBeenCalled();
    expect(useUiStore().banner?.message).toBe(
      'Save or discard the unsaved document before deleting this project.',
    );
  });

  it('awaits deletion then clears project caches, tabs, last view, and active route together', async () => {
    const workspace = seedProject();
    const remove = vi.spyOn(workspace, 'deleteProject').mockResolvedValue(true);
    const documents = useDocumentsStore();
    documents.publishSummariesForProject('atlas', [
      {
        id: 'doc-id',
        slug: 'doc-1',
        title: 'Document',
        folder_id: null,
        head_seq: 1,
        updated_at: '2026-01-01T00:00:00Z',
      },
    ]);
    const boards = useBoardsStore();
    boards.publishForProject('atlas', [
      {
        id: 'board-1',
        name: 'Board',
        task_count: 0,
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    ]);
    const tabs = useNotesTabsStore();
    tabs.open('acme', { kind: 'doc', id: 'doc-1' }, 'Document', 'atlas');
    tabs.open('acme', { kind: 'board', id: 'board-1' }, 'Board', 'atlas');
    useLastViewedStore().record('acme', { name: 'notes', params: { slug: 'doc-1' } });

    const deleted = await useProjectDeletion().deleteProject(project);

    expect(deleted).toBe(true);
    expect(remove).toHaveBeenCalledWith('acme', 'atlas');
    expect(purgeTags).toHaveBeenCalledWith(
      expect.arrayContaining(['workspace:workspace-id', 'project:atlas', 'document:doc-1', 'board:board-1']),
      'user:principal',
      'workspace-id',
    );
    expect(documents.summariesFor('atlas')).toEqual([]);
    expect(boards.boardsFor('atlas')).toEqual([]);
    expect(tabs.tabs('acme')).toEqual([]);
    expect(useLastViewedStore().forWorkspace('acme')).toBeNull();
    expect(push).toHaveBeenCalledWith({ name: 'notes' });
  });

  it('surfaces an awaited delete failure without clearing local project state', async () => {
    const workspace = seedProject();
    vi.spyOn(workspace, 'deleteProject').mockImplementation(async () => {
      workspace.error = 'Delete failed';
      return false;
    });
    const tabs = useNotesTabsStore();
    tabs.open('acme', { kind: 'doc', id: 'doc-1' }, 'Document', 'atlas');

    const deleted = await useProjectDeletion().deleteProject(project);

    expect(deleted).toBe(false);
    expect(tabs.tabs('acme')).toEqual([
      { kind: 'doc', id: 'doc-1', title: 'Document', projectSlug: 'atlas' },
    ]);
    expect(useUiStore().banner?.message).toBe('Delete failed');
  });
});
