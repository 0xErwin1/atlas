import { describe, expect, it } from 'vitest';
import { activityPhrase } from '@/lib/activityPhrase';

describe('activityPhrase', () => {
  it('renders unit-variant verbs from the kind alone', () => {
    expect(activityPhrase('created', 'created')).toBe('created');
    expect(activityPhrase('deleted', 'deleted')).toBe('deleted');
    expect(activityPhrase('moved', { moved: { from_column_id: 'a', to_column_id: 'b' } })).toBe('moved');
  });

  it('enriches field_changed with the changed field name from the payload', () => {
    const payload = { field_changed: { field: 'priority', old_value: 'low', new_value: 'high' } };
    expect(activityPhrase('field_changed', payload)).toBe('changed priority on');
  });

  it('falls back when field_changed payload lacks a usable field', () => {
    expect(activityPhrase('field_changed', { field_changed: {} })).toBe('changed a field on');
    expect(activityPhrase('field_changed', null)).toBe('changed a field on');
  });

  it('enriches checklist_added with the item title when present', () => {
    const payload = { checklist_added: { item_id: 'i1', title: 'Write tests' } };
    expect(activityPhrase('checklist_added', payload)).toBe('added the checklist item "Write tests" to');
  });

  it('uses readable phrases for reference and assignment verbs', () => {
    expect(activityPhrase('assigned', { assigned: { assignee: {} } })).toBe('assigned');
    expect(activityPhrase('unassigned', { unassigned: { assignee: {} } })).toBe(
      'unassigned an assignee from',
    );
    expect(activityPhrase('reference_added', {})).toBe('added a reference to');
    expect(activityPhrase('reference_removed', {})).toBe('removed a reference from');
  });

  it('enriches document_mentioned with the document title when present', () => {
    const payload = { document_mentioned: { document_id: 'd1', title: 'Design Doc' } };
    expect(activityPhrase('document_mentioned', payload)).toBe('referenced the document "Design Doc" in');
  });

  it('falls back when document_mentioned payload lacks a title', () => {
    expect(activityPhrase('document_mentioned', { document_mentioned: { document_id: 'd1' } })).toBe(
      'referenced a document in',
    );
  });

  it('humanises an unknown kind rather than rendering a raw token', () => {
    expect(activityPhrase('some_new_verb', {})).toBe('some new verb');
    expect(activityPhrase('', null)).toBe('updated this task');
  });

  it('ignores non-string payload fields (defensive against untrusted input)', () => {
    const payload = { field_changed: { field: 42 } };
    expect(activityPhrase('field_changed', payload)).toBe('changed a field on');
  });
});
