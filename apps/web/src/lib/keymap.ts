export type ShortcutScope = 'overlay' | 'task' | 'board' | 'global';
export type ShortcutId = 'command-palette' | 'board-search' | 'shortcuts-help' | 'escape';

export interface ShortcutMeta {
  id: ShortcutId;
  scope: ShortcutScope;
  keys: string[];
  label: string;
}

export interface RegisteredShortcutHandler {
  id: ShortcutId;
  priority: number;
  order: number;
  allowInText?: boolean;
  blockInMarkdown?: boolean;
  handler: (event: KeyboardEvent) => boolean | undefined;
}

export const KEYMAP_PRIORITIES = {
  global: 10,
  board: 20,
  task: 30,
  overlay: 40,
} as const;

export const shortcutCatalog: ShortcutMeta[] = [
  { id: 'command-palette', scope: 'global', keys: ['mod+k'], label: 'Command palette' },
  { id: 'board-search', scope: 'board', keys: ['/'], label: 'Focus board search' },
  { id: 'shortcuts-help', scope: 'global', keys: ['shift+?'], label: 'Keyboard shortcuts' },
  { id: 'escape', scope: 'global', keys: ['escape'], label: 'Dismiss or go back' },
];

export function getShortcutCatalog(): ShortcutMeta[] {
  return shortcutCatalog.map((shortcut) => ({ ...shortcut, keys: [...shortcut.keys] }));
}

export function matchesShortcut(event: KeyboardEvent, shortcut: ShortcutMeta): boolean {
  if (event.isComposing) return false;

  return shortcut.keys.some((key) => matchesKeyToken(event, key));
}

export function isTextEntryTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (isMarkdownEditorTarget(target)) return true;

  const selfContentEditable =
    'contentEditable' in target ? String(target.contentEditable).toLowerCase() : 'inherit';
  if (selfContentEditable === 'true' || selfContentEditable === '') return true;

  const editable = target.closest('[contenteditable]');
  if (editable !== null && editable.getAttribute('contenteditable')?.toLowerCase() !== 'false') return true;

  const textEntry = target.closest('input, textarea, select, [data-keymap-text-entry]');
  return textEntry !== null;
}

export function isMarkdownEditorTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest('.cm-editor, .cm-content') !== null;
}

export function pickFirstShortcutHandler(
  event: KeyboardEvent,
  handlers: readonly RegisteredShortcutHandler[],
): RegisteredShortcutHandler | null {
  return matchingShortcutHandlers(event, handlers)[0] ?? null;
}

export function matchingShortcutHandlers(
  event: KeyboardEvent,
  handlers: readonly RegisteredShortcutHandler[],
): RegisteredShortcutHandler[] {
  if (event.defaultPrevented || event.isComposing) return [];

  const matches = handlers.filter((handler) => handlerMatchesEvent(event, handler));
  return matches.sort((left, right) => {
    const priority = right.priority - left.priority;
    if (priority !== 0) return priority;
    return right.order - left.order;
  });
}

function handlerMatchesEvent(event: KeyboardEvent, handler: RegisteredShortcutHandler): boolean {
  const shortcut = shortcutCatalog.find((entry) => entry.id === handler.id);
  if (shortcut === undefined || !matchesShortcut(event, shortcut)) return false;

  if (handler.blockInMarkdown === true && isMarkdownEditorTarget(event.target)) return false;
  if (handler.allowInText !== true && isTextEntryTarget(event.target)) return false;

  return true;
}

function matchesKeyToken(event: KeyboardEvent, token: string): boolean {
  const parts = token.toLowerCase().split('+');
  const key = parts[parts.length - 1] ?? '';
  const wantsShift = parts.includes('shift');
  const wantsMod = parts.includes('mod');
  const wantsCtrl = parts.includes('ctrl');
  const wantsMeta = parts.includes('meta');
  const wantsAlt = parts.includes('alt');

  const modMatches = wantsMod
    ? event.metaKey || event.ctrlKey
    : event.metaKey === wantsMeta && event.ctrlKey === wantsCtrl;
  if (!modMatches) return false;
  if (event.shiftKey !== wantsShift) return false;
  if (event.altKey !== wantsAlt) return false;

  return normalizeEventKey(event.key) === key;
}

function normalizeEventKey(key: string): string {
  if (key === 'Esc') return 'escape';
  return key.toLowerCase();
}
