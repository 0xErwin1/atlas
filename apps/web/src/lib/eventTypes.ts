// Mirrors the server-side catalog (crates/atlas_domain/src/entities/events.rs
// event_type() and routes/webhooks.rs KNOWN_EVENT_TYPES). Keep in sync.
export const EVENT_TYPE = {
  PROJECT_CREATED: 'project.created',
  TASK_CREATED: 'task.created',
  TASK_UPDATED: 'task.updated',
  TASK_MOVED: 'task.moved',
  TASK_DELETED: 'task.deleted',
  DOCUMENT_CREATED: 'document.created',
  DOCUMENT_UPDATED: 'document.updated',
  DOCUMENT_MOVED: 'document.moved',
  DOCUMENT_DELETED: 'document.deleted',
  BOARD_CREATED: 'board.created',
  BOARD_UPDATED: 'board.updated',
  BOARD_DELETED: 'board.deleted',
  BOARD_MOVED: 'board.moved',
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

/** The principal that produced an event: a `user` or an `api_key`, with its id. */
export interface LiveActor {
  type: string;
  id: string;
}

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
  document_id?: string | null;
  occurred_at: string;
  actor: LiveActor;
  data: unknown;
}

/** Reads a string field from an event payload, or undefined when absent/non-string. */
export function eventString(data: unknown, key: string): string | undefined {
  if (typeof data !== 'object' || data === null) return undefined;

  const value = (data as Record<string, unknown>)[key];
  return typeof value === 'string' ? value : undefined;
}

/** Renders an actor as the `type:id` key the auth store derives from `MeResponse`. */
export function actorKey(actor: LiveActor): string {
  return `${actor.type}:${actor.id}`;
}

/**
 * True when this frame was produced by the principal this client is signed in as.
 *
 * The wire carries no per-session identity, only the acting principal, so a
 * second tab or device signed in as the same user counts as self. An unknown
 * session is never self, so an unauthenticated or still-loading client keeps
 * applying every frame it receives.
 *
 * The envelope reaches here as an unvalidated `JSON.parse` cast, so an actor the
 * server never sent must read as "not self" rather than throw inside the stream
 * dispatch and take live updates down for the whole workspace.
 */
export function isSelfActor(envelope: LiveEnvelope, sessionActor: string | null | undefined): boolean {
  if (sessionActor === null || sessionActor === undefined || sessionActor === '') return false;

  const actor: Partial<LiveActor> | null | undefined = envelope.actor;
  if (typeof actor?.type !== 'string' || typeof actor.id !== 'string') return false;

  return actorKey(actor as LiveActor) === sessionActor;
}
