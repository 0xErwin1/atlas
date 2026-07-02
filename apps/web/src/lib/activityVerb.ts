const VERB: Record<string, string> = {
  created: 'created this task',
  moved: 'moved the task',
  assigned: 'assigned the task',
  unassigned: 'unassigned the task',
  field_changed: 'changed a field',
  reference_added: 'added a reference',
  reference_removed: 'removed a reference',
  checklist_added: 'added a checklist item',
  checklist_updated: 'updated a checklist item',
  checklist_removed: 'removed a checklist item',
  checklist_promoted: 'promoted a checklist item to a task',
  document_mentioned: 'referenced a document',
  deleted: 'deleted the task',
};

/** Human-readable phrase for an activity entry kind; falls back to the raw kind. */
export function activityVerb(kind: string): string {
  return VERB[kind] ?? kind;
}
