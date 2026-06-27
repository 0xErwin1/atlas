import { ref } from 'vue';
import { useRouter } from 'vue-router';
import type { MenuItem } from '@/components/ui/ContextMenu.vue';
import { AI_ACTIONS } from '@/lib/aiPrompt';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const PRIORITIES = ['urgent', 'high', 'medium', 'low'] as const;

/**
 * Context passed to buildMenuItems. Separates concerns between the single-board
 * case (boardId from the store) and the cross-board case (boardId from task.board_id),
 * so the same builder works in KanbanBoard, TaskListView, and TaskViewListView.
 */
export interface TaskMenuCtx {
  /** The task object directly — not looked up via boards.findTaskByReadableId. */
  task: TaskSummaryDto;
  /** The board this task lives on. Single-board: boards.board?.id. Cross-board: task.board_id. */
  boardId: string | undefined;
  /** Status submenu source for THIS task's board (may be lazily loaded). */
  columns: ColumnDto[];
  /** True when boardId is known and Duplicate should be enabled. */
  allowDuplicate: boolean;
  /** Called when the user picks "Open". */
  onOpen: (readableId: string) => void;
  /**
   * Called when the user picks "Open as…" → a specific presentation. Opens the
   * task in `mode` for this one time only, without changing the saved default.
   * Optional: hosts without an inline pane may omit it (the submenu is hidden).
   */
  onOpenAs?: (readableId: string, mode: TaskViewMode) => void;
}

/**
 * Extracts all task interaction handlers and the context-menu builder from
 * KanbanBoard into a reusable composable. Hosts (KanbanBoard, TaskListView,
 * TaskViewListView) call this, bind its dialog state refs, and pass ctx into
 * buildMenuItems at menu-open time.
 *
 * Dialog state (promptState, confirmOpen) is owned here and returned so each
 * host can render its own PromptDialog / ConfirmDialog bound to these refs.
 */
export function useTaskInteractions(ws: string) {
  const boards = useBoardsStore();
  const workspace = useWorkspaceStore();
  const ui = useUiStore();
  const router = useRouter();

  const menuReadableId = ref<string | null>(null);

  const promptState = ref<{
    open: boolean;
    mode: 'rename' | 'due';
    title: string;
    initial: string;
  }>({
    open: false,
    mode: 'rename',
    title: '',
    initial: '',
  });

  const confirmOpen = ref(false);

  function taskHref(readableId: string): string {
    return router.resolve({ name: 'task-detail', params: { readableId } }).href;
  }

  async function runUpdate(
    readableId: string,
    patch: Parameters<typeof boards.updateTask>[2],
  ): Promise<void> {
    const ok = await boards.updateTask(ws, readableId, patch);
    if (!ok && boards.error) ui.showBanner(boards.error, 'error');
  }

  async function runMoveToColumn(readableId: string, columnId: string): Promise<void> {
    const ok = await boards.moveTaskToColumn(ws, readableId, columnId);
    if (!ok && boards.error) ui.showBanner(boards.error, 'error');
  }

  async function runMoveToBoard(readableId: string, boardId: string): Promise<void> {
    const ok = await boards.moveTaskToBoard(ws, readableId, boardId);
    if (ok) ui.showBanner('Task moved to board', 'success');
    else if (boards.error) ui.showBanner(boards.error, 'error');
  }

  async function runAssign(readableId: string, type: string, id: string): Promise<void> {
    const ok = await boards.assignTask(ws, readableId, type, id);
    if (ok) ui.showBanner('Task assigned', 'success');
    else if (boards.error) ui.showBanner(boards.error, 'error');
  }

  async function runUnassign(readableId: string, type: string, id: string): Promise<void> {
    const ok = await boards.unassignTask(ws, readableId, type, id);
    if (ok) ui.showBanner('Task unassigned', 'success');
    else if (boards.error) ui.showBanner(boards.error, 'error');
  }

  async function runDuplicate(readableId: string, boardId: string): Promise<void> {
    const created = await boards.duplicateTask(ws, boardId, readableId);
    if (created === null && boards.error) ui.showBanner(boards.error, 'error');
  }

  async function copyText(text: string, label: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      ui.showBanner(`${label} copied`, 'success');
    } catch {
      ui.showBanner('Clipboard is not available', 'error');
    }
  }

  function openRename(task: { title: string }): void {
    promptState.value = { open: true, mode: 'rename', title: 'Rename task', initial: task.title };
  }

  function openDueDate(): void {
    promptState.value = { open: true, mode: 'due', title: 'Set due date', initial: '' };
  }

  async function onPromptConfirm(value: string): Promise<void> {
    const readableId = menuReadableId.value;
    const mode = promptState.value.mode;
    promptState.value = { ...promptState.value, open: false };
    if (readableId === null) return;

    if (mode === 'rename') {
      const title = value.trim();
      if (title.length > 0) await runUpdate(readableId, { title });
      return;
    }

    const due = value === '' ? null : new Date(value).toISOString();
    await runUpdate(readableId, { due_date: due });
  }

  async function onConfirmDelete(): Promise<void> {
    const readableId = menuReadableId.value;
    confirmOpen.value = false;
    if (readableId === null) return;

    const ok = await boards.deleteTask(ws, readableId);
    if (!ok && boards.error) ui.showBanner(boards.error, 'error');
  }

  function deleteTargetFor(readableId: string | null): TaskSummaryDto | null {
    if (readableId === null) return null;
    return boards.findTaskByReadableId(readableId) ?? null;
  }

  /**
   * Build the context menu item list for a given task and its board context.
   * Accepts the task object and columns explicitly so it works for both single-board
   * (where boards.findTaskByReadableId is available) and cross-board row contexts
   * (where the task lives in a different store).
   */
  function buildMenuItems(ctx: TaskMenuCtx): MenuItem[] {
    const { task, boardId, columns, allowDuplicate, onOpen, onOpenAs } = ctx;
    const readableId = task.readable_id;

    const statusChildren = buildStatusChildren(task, columns, readableId);
    const priorityChildren = buildPriorityChildren(task, readableId);
    const assignChildren = buildAssignChildren(readableId);
    const moveChildren = buildMoveChildren(boardId, readableId);

    return [
      { label: 'Open', icon: 'square-arrow-out-up-right', action: () => onOpen(readableId) },
      ...(onOpenAs !== undefined ? [buildOpenAsItem(readableId, onOpenAs)] : []),
      {
        label: 'Open in new tab',
        icon: 'external-link',
        action: () => window.open(`${window.location.origin}${taskHref(readableId)}`, '_blank'),
      },
      buildAskAiItem(task),
      { sep: true },
      { label: 'Rename', icon: 'pencil', action: () => openRename(task) },
      { label: 'Change status', icon: 'square-kanban', children: statusChildren },
      { label: 'Change priority', icon: 'flag', children: priorityChildren },
      { label: 'Assign to', icon: 'user-plus', children: assignChildren },
      { label: 'Move to board', icon: 'arrow-right-left', children: moveChildren },
      { label: 'Set due date', icon: 'calendar', action: openDueDate },
      { sep: true },
      { label: 'Copy ID', icon: 'hash', action: () => copyText(readableId, 'ID') },
      {
        label: 'Copy link',
        icon: 'link',
        action: () => copyText(`${window.location.origin}${taskHref(readableId)}`, 'Link'),
      },
      {
        label: 'Duplicate',
        icon: 'copy',
        disabled: !allowDuplicate,
        action: allowDuplicate && boardId !== undefined ? () => runDuplicate(readableId, boardId) : undefined,
      },
      { sep: true },
      { label: 'Delete', icon: 'trash-2', danger: true, action: () => (confirmOpen.value = true) },
    ];
  }

  /**
   * "Open as…" submenu: opens the task in a chosen presentation for this time
   * only. The mode order mirrors the persisted TaskViewModeSwitch (dock,
   * dialog, full) so the icons read the same across the app.
   */
  function buildOpenAsItem(
    readableId: string,
    onOpenAs: (readableId: string, mode: TaskViewMode) => void,
  ): MenuItem {
    return {
      label: 'Open as…',
      icon: 'panel-right',
      children: [
        { label: 'Side panel', icon: 'panel-right', action: () => onOpenAs(readableId, 'sidebar') },
        { label: 'Floating dialog', icon: 'app-window', action: () => onOpenAs(readableId, 'modal') },
        { label: 'Full screen', icon: 'maximize', action: () => onOpenAs(readableId, 'full') },
      ],
    };
  }

  /**
   * "Ask AI" submenu: opens the global hand-off dialog pre-set to an action. The
   * board summary supplies the status (column_name) and the fields the prompt
   * builder needs; the description is filled in only on the task detail banner.
   */
  function buildAskAiItem(task: TaskSummaryDto): MenuItem {
    return {
      label: 'Ask AI',
      icon: 'sparkles',
      children: AI_ACTIONS.map((a) => ({
        label: a.label,
        icon: a.icon,
        action: () => ui.openAskAi(task, task.column_name, a.value),
      })),
    };
  }

  function buildStatusChildren(task: TaskSummaryDto, columns: ColumnDto[], readableId: string): MenuItem[] {
    return columns.map((c) => ({
      label: c.name,
      disabled: c.id === task.column_id,
      action: () => runMoveToColumn(readableId, c.id),
    }));
  }

  function buildPriorityChildren(task: TaskSummaryDto, readableId: string): MenuItem[] {
    return [
      ...PRIORITIES.map((p) => ({
        label: p.charAt(0).toUpperCase() + p.slice(1),
        disabled: task.priority === p,
        action: () => runUpdate(readableId, { priority: p }),
      })),
      { sep: true },
      { label: 'Clear', action: () => runUpdate(readableId, { priority: null }) },
    ];
  }

  function buildAssignChildren(readableId: string): MenuItem[] {
    return workspace.members.length > 0
      ? workspace.members.map((m) => ({
          label: m.display,
          icon: m.principal_type === 'api_key' ? 'bot' : 'user',
          action: () => runAssign(readableId, m.principal_type, m.id),
        }))
      : [{ label: 'No members', disabled: true }];
  }

  function buildMoveChildren(boardId: string | undefined, readableId: string): MenuItem[] {
    const otherBoards: MenuItem[] = workspace.projects
      .flatMap((p) => boards.boardsFor(p.slug).map((b) => ({ board: b, projectName: p.name })))
      .filter((entry) => entry.board.id !== boardId)
      .map((entry) => ({
        label: `${entry.projectName} / ${entry.board.name}`,
        action: () => runMoveToBoard(readableId, entry.board.id),
      }));

    return otherBoards.length > 0 ? otherBoards : [{ label: 'No other boards', disabled: true }];
  }

  return {
    menuReadableId,
    promptState,
    confirmOpen,
    buildMenuItems,
    runUpdate,
    runMoveToColumn,
    runMoveToBoard,
    runAssign,
    runUnassign,
    runDuplicate,
    copyText,
    openRename,
    openDueDate,
    onPromptConfirm,
    onConfirmDelete,
    deleteTargetFor,
  };
}
