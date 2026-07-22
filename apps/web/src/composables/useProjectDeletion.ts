import { useRouter } from 'vue-router';
import { getResourceCachePrincipal, resourceCache } from '@/cache/cacheRuntime';
import { routeAfterClose } from '@/lib/docsTabs';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useLastViewedStore } from '@/stores/lastViewed';
import { type TabRef, useNotesTabsStore } from '@/stores/notesTabs';
import { useUiStore } from '@/stores/ui';
import { type ProjectSummary, useWorkspaceStore } from '@/stores/workspace';

function sameRef(a: TabRef, b: TabRef): boolean {
  return a.kind === b.kind && a.id === b.id;
}

/**
 * Deletes a project through the single client-side lifecycle boundary shared by
 * settings and the sidebar. It refuses to delete unsaved project documents,
 * then reconciles the affected cache, catalogs, tabs, last-viewed entry and route.
 */
export function useProjectDeletion() {
  const router = useRouter();
  const workspace = useWorkspaceStore();
  const documents = useDocumentsStore();
  const folders = useFoldersStore();
  const boards = useBoardsStore();
  const tabs = useNotesTabsStore();
  const lastViewed = useLastViewedStore();
  const ui = useUiStore();

  async function deleteProject(project: ProjectSummary): Promise<boolean> {
    const ws = workspace.activeWorkspaceSlug;
    if (ws === null) return false;

    const documentRefs: TabRef[] = documents
      .summariesFor(project.slug)
      .flatMap((document) =>
        typeof document.slug === 'string' ? [{ kind: 'doc' as const, id: document.slug }] : [],
      );
    const legacyRefs: TabRef[] = [
      ...documentRefs,
      ...boards.boardsFor(project.slug).map((board) => ({ kind: 'board' as const, id: board.id })),
    ];
    if (tabs.hasDirtyProjectDocument(ws, project.slug, legacyRefs)) {
      ui.showBanner('Save or discard the unsaved document before deleting this project.', 'error');
      return false;
    }

    const deleted = await workspace.deleteProject(ws, project.slug);
    if (!deleted) {
      ui.showBanner(workspace.error ?? 'Failed to delete project', 'error');
      return false;
    }

    const removedTabs = tabs.closeProject(ws, project.slug, legacyRefs);
    const resourceTags = removedTabs.map((tab) => `${tab.kind === 'doc' ? 'document' : 'board'}:${tab.id}`);
    const workspaceId = workspace.workspaceIdForSlug(ws);
    if (workspaceId !== null) {
      try {
        const invalidated = await resourceCache.purgeTags(
          [`workspace:${workspaceId}`, `project:${project.slug}`, ...resourceTags],
          getResourceCachePrincipal(),
          workspaceId,
        );
        if (!invalidated)
          ui.showBanner('Project deleted, but cached resources could not be cleared.', 'error');
      } catch {
        ui.showBanner('Project deleted, but cached resources could not be cleared.', 'error');
      }
    }

    documents.clearProject(project.slug);
    folders.clearProject(project.slug);
    boards.clearProject(project.slug, project.id);

    const current = router.currentRoute.value;
    const currentRef =
      current.name === 'notes' && typeof current.params.slug === 'string'
        ? { kind: 'doc' as const, id: current.params.slug }
        : current.name === 'tasks' && typeof current.params.boardId === 'string'
          ? { kind: 'board' as const, id: current.params.boardId }
          : null;
    const routeDeleted = currentRef !== null && removedTabs.some((tab) => sameRef(tab, currentRef));
    const lastViewedTarget = lastViewed.forWorkspace(ws);
    const lastViewedDeleted =
      lastViewedTarget !== null &&
      removedTabs.some(
        (tab) =>
          (tab.kind === 'doc' &&
            lastViewedTarget.name === 'notes' &&
            lastViewedTarget.params.slug === tab.id) ||
          (tab.kind === 'board' &&
            lastViewedTarget.name === 'tasks' &&
            lastViewedTarget.params.boardId === tab.id),
      );
    if (lastViewedDeleted) lastViewed.clear(ws);

    if (routeDeleted) await router.push(routeAfterClose(tabs.tabs(ws)[0] ?? null));
    return true;
  }

  return { deleteProject };
}
