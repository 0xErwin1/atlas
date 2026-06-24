/**
 * Render a workspace activity entry into a short, human-readable phrase.
 *
 * The `kind` discriminant drives the verb; `payload` is read defensively (it is
 * untrusted `unknown` from the wire and its shape varies per verb) and only used
 * to enrich the phrase when a known, string-typed field is present. Unknown
 * kinds fall back to a humanised form of the verb so the feed never renders an
 * empty or raw token.
 */

type Payload = unknown;

function asRecord(payload: Payload): Record<string, unknown> | null {
  if (typeof payload !== 'object' || payload === null) return null;
  return payload as Record<string, unknown>;
}

function stringField(payload: Payload, key: string): string | null {
  const record = asRecord(payload);
  const value = record?.[key];
  return typeof value === 'string' && value.length > 0 ? value : null;
}

// A unit-variant payload serialises to a bare string ("created"); a data-bearing
// variant serialises externally-tagged ({ "field_changed": { ... } }). Reach the
// inner object for the entry's own kind, when present.
function variantData(kind: string, payload: Payload): Payload {
  const record = asRecord(payload);
  if (record === null) return null;
  return record[kind] ?? null;
}

function humaniseKind(kind: string): string {
  const spaced = kind.replace(/_/g, ' ').trim();
  if (spaced === '') return 'updated this task';
  return spaced;
}

/**
 * Returns the action phrase WITHOUT the actor or the task reference — callers
 * compose "{actor} {phrase} {task-link}". The phrase reads naturally after an
 * actor name, e.g. "created", "moved", "changed the priority".
 */
export function activityPhrase(kind: string, payload: Payload): string {
  switch (kind) {
    case 'created':
      return 'created';
    case 'deleted':
      return 'deleted';
    case 'moved':
      return 'moved';
    case 'assigned':
      return 'assigned';
    case 'unassigned':
      return 'unassigned an assignee from';
    case 'field_changed': {
      const field = stringField(variantData(kind, payload), 'field');
      return field !== null ? `changed ${field} on` : 'changed a field on';
    }
    case 'reference_added':
      return 'added a reference to';
    case 'reference_removed':
      return 'removed a reference from';
    case 'checklist_added': {
      const title = stringField(variantData(kind, payload), 'title');
      return title !== null ? `added the checklist item "${title}" to` : 'added a checklist item to';
    }
    case 'checklist_updated':
      return 'updated a checklist item on';
    case 'checklist_removed':
      return 'removed a checklist item from';
    case 'checklist_promoted':
      return 'promoted a checklist item on';
    default:
      return humaniseKind(kind);
  }
}
