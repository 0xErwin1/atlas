/** Board layout presentation shared by the board view switcher and the settings preference. */

import type { TaskBoardView } from '@/stores/ui';

export interface BoardViewOption {
  id: TaskBoardView;
  label: string;
  icon: string;
}

export const DEFAULT_BOARD_VIEW: BoardViewOption = { id: 'board', label: 'Board', icon: 'columns-3' };

export const BOARD_VIEWS: BoardViewOption[] = [
  DEFAULT_BOARD_VIEW,
  { id: 'list', label: 'List', icon: 'tasks' },
  { id: 'table', label: 'Table', icon: 'dashboard' },
  { id: 'calendar', label: 'Calendar', icon: 'calendar' },
  { id: 'timeline', label: 'Timeline', icon: 'clock' },
];

export function isBoardView(value: unknown): value is TaskBoardView {
  return BOARD_VIEWS.some((view) => view.id === value);
}
