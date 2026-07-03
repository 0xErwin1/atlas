// Mirrors the server-side catalog (crates/atlas_domain/src/entities/events.rs
// event_type() and routes/webhooks.rs KNOWN_EVENT_TYPES). Keep in sync.
export const EVENT_TYPES = [
  'task.created',
  'task.updated',
  'task.moved',
  'task.deleted',
  'document.created',
  'document.updated',
  'document.moved',
  'document.deleted',
  'board.created',
  'board.deleted',
  'column.created',
  'column.deleted',
  'folder.created',
  'folder.deleted',
] as const;
