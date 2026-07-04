// Mirrors the server-side catalog (crates/atlas_domain/src/entities/events.rs
// event_type() and routes/webhooks.rs KNOWN_EVENT_TYPES). Keep in sync.
export const EVENT_TYPE = {
  TASK_CREATED: 'task.created',
  TASK_UPDATED: 'task.updated',
  TASK_MOVED: 'task.moved',
  TASK_DELETED: 'task.deleted',
  DOCUMENT_CREATED: 'document.created',
  DOCUMENT_UPDATED: 'document.updated',
  DOCUMENT_MOVED: 'document.moved',
  DOCUMENT_DELETED: 'document.deleted',
  BOARD_CREATED: 'board.created',
  BOARD_DELETED: 'board.deleted',
  COLUMN_CREATED: 'column.created',
  COLUMN_DELETED: 'column.deleted',
  FOLDER_CREATED: 'folder.created',
  FOLDER_DELETED: 'folder.deleted',
} as const;

export type EventType = (typeof EVENT_TYPE)[keyof typeof EVENT_TYPE];

// The flat catalog, preserving declaration order for the webhook event picker.
export const EVENT_TYPES: readonly EventType[] = Object.values(EVENT_TYPE);

// Streamed over SSE but deliberately outside EVENT_TYPE/EVENT_TYPES: these are
// live-only and absent from the server's webhook catalog, so they must not reach
// the webhook event picker while still needing a named SSE listener.
export const PRESENCE_UPDATED = 'presence.updated';

export const LIVE_ONLY_EVENT_TYPES: readonly string[] = [PRESENCE_UPDATED];

/**
 * The full domain-event envelope streamed over SSE. `data` is the per-type
 * payload (see the wire contract); it is left as `unknown` here and read through
 * `eventString` so a consumer never assumes a shape the server did not send.
 */
export interface LiveEnvelope {
  id: string;
  event_type: string;
  version: number;
  source: string;
  workspace_id: string;
  project_id?: string | null;
  board_id?: string | null;
  occurred_at: string;
  actor: { type: string; id: string };
  data: unknown;
}

/** Reads a string field from an event payload, or undefined when absent/non-string. */
export function eventString(data: unknown, key: string): string | undefined {
  if (typeof data !== 'object' || data === null) return undefined;

  const value = (data as Record<string, unknown>)[key];
  return typeof value === 'string' ? value : undefined;
}
