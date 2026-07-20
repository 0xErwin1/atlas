import { type ComputedRef, computed } from 'vue';
import { useRoute } from 'vue-router';
import { useTasksStore } from '@/stores/tasks';

/**
 * Derives which sidebar node the current route highlights, mapping each of the
 * unified sidebar's live routes to a single selected node:
 *   - `/n/:slug`            -> the document row (`activeSlug`)
 *   - `/t/:boardId`         -> the board row (`activeBoardId`)
 *   - `/t/views/:viewId`    -> the view row in SidebarViews (`activeViewId`)
 *   - `/t/task/:readableId` -> the task's parent board row (`activeBoardId`)
 *
 * On the task-detail route the parent board is resolved from the open task in
 * the tasks store; the highlight stays empty until the loaded task matches the
 * route's readable id, so it resolves once the task becomes available.
 */
export function useActiveSidebarNode(): {
  activeSlug: ComputedRef<string | null>;
  activeBoardId: ComputedRef<string | null>;
  activeViewId: ComputedRef<string | null>;
} {
  const route = useRoute();
  const tasks = useTasksStore();

  const activeSlug = computed(() => {
    const slug = route.params.slug;
    return typeof slug === 'string' && slug.length > 0 ? slug : null;
  });

  const activeViewId = computed(() => {
    const viewId = route.params.viewId;
    return typeof viewId === 'string' && viewId.length > 0 ? viewId : null;
  });

  const activeBoardId = computed(() => {
    const boardId = route.params.boardId;
    if (typeof boardId === 'string' && boardId.length > 0) return boardId;

    const readableId = route.params.readableId;
    if (typeof readableId !== 'string' || readableId.length === 0) return null;

    const open = tasks.openTask;
    if (open === null || open.readable_id !== readableId) return null;

    return typeof open.board_id === 'string' && open.board_id.length > 0 ? open.board_id : null;
  });

  return { activeSlug, activeBoardId, activeViewId };
}
