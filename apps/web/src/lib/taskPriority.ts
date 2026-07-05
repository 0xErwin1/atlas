/** Priority presentation shared by the task list view and its row component. */

export const PRIORITY_COLOR: Record<string, string> = {
  urgent: 'var(--c-danger)',
  high: 'var(--c-primary)',
  medium: 'var(--c-info)',
  low: 'var(--c-muted)',
};

export function priorityLabel(priority: string | null): string {
  if (priority === null || priority === '') return 'No priority';
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}
