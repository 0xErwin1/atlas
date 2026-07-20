import { describe, expect, it } from 'vitest';
import { noteCatalogSchema } from '@/lib/noteCatalog';

const folder = {
  id: 'f1',
  name: 'Sprint',
  parent_folder_id: null,
  project_id: 'p1',
  workspace_id: 'w1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

const summary = {
  id: 'd1',
  slug: 'doc',
  title: 'Doc',
  folder_id: null,
  head_seq: 1,
  updated_at: '2026-01-01T00:00:00Z',
};

const boardEntry = {
  id: 'b1',
  name: 'Backlog',
  folder_id: 'f1',
  task_count: 4,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

describe('noteCatalogSchema', () => {
  it('parses a catalog payload that includes boards', () => {
    const parsed = noteCatalogSchema.parse({ folders: [folder], summaries: [summary], boards: [boardEntry] });

    expect(parsed.boards).toHaveLength(1);
    expect(parsed.boards[0]?.id).toBe('b1');
    expect(parsed.boards[0]?.task_count).toBe(4);
    expect(parsed.boards[0]?.folder_id).toBe('f1');
  });

  it('defaults boards to an empty array for a legacy cached payload without the field, without throwing', () => {
    const parsed = noteCatalogSchema.parse({ folders: [folder], summaries: [summary] });

    expect(parsed.boards).toEqual([]);
  });

  it('keeps unknown board fields via passthrough, matching the folder/document summary convention', () => {
    const parsed = noteCatalogSchema.parse({
      folders: [folder],
      summaries: [summary],
      boards: [{ ...boardEntry, project_id: 'p1', workspace_id: 'w1' }],
    });

    expect((parsed.boards[0] as { project_id?: string }).project_id).toBe('p1');
  });
});
