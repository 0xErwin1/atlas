import { describe, expect, it } from 'vitest';
import { buildNotesTree } from '@/lib/notesTree';

describe('buildNotesTree', () => {
  it('places root folders and root docs at the top level', () => {
    const tree = buildNotesTree(
      [{ id: 'f1', name: 'Specs', parent_folder_id: null }],
      [{ id: 'd1', title: 'Readme', slug: 'readme', folder_id: null }],
    );

    expect(tree.folders).toHaveLength(1);
    expect(tree.folders[0]?.id).toBe('f1');
    expect(tree.docs).toHaveLength(1);
    expect(tree.docs[0]?.slug).toBe('readme');
  });

  it('nests folders by parent_folder_id', () => {
    const tree = buildNotesTree(
      [
        { id: 'f1', name: 'Root', parent_folder_id: null },
        { id: 'f2', name: 'Child', parent_folder_id: 'f1' },
      ],
      [],
    );

    expect(tree.folders).toHaveLength(1);
    expect(tree.folders[0]?.folders).toHaveLength(1);
    expect(tree.folders[0]?.folders[0]?.id).toBe('f2');
  });

  it('nests docs inside their folder by folder_id', () => {
    const tree = buildNotesTree(
      [{ id: 'f1', name: 'Notes', parent_folder_id: null }],
      [{ id: 'd1', title: 'Inside', slug: 'inside', folder_id: 'f1' }],
    );

    expect(tree.folders[0]?.docs).toHaveLength(1);
    expect(tree.folders[0]?.docs[0]?.id).toBe('d1');
    expect(tree.docs).toHaveLength(0);
  });

  it('sorts folders and docs case-insensitively by name/title', () => {
    const tree = buildNotesTree(
      [
        { id: 'f2', name: 'beta', parent_folder_id: null },
        { id: 'f1', name: 'Alpha', parent_folder_id: null },
      ],
      [
        { id: 'd2', title: 'zeta', slug: 'zeta', folder_id: null },
        { id: 'd1', title: 'Apple', slug: 'apple', folder_id: null },
      ],
    );

    expect(tree.folders.map((f) => f.id)).toEqual(['f1', 'f2']);
    expect(tree.docs.map((d) => d.id)).toEqual(['d1', 'd2']);
  });

  it('reparents to root when the parent folder is absent from the input', () => {
    const tree = buildNotesTree(
      [{ id: 'f2', name: 'Orphan', parent_folder_id: 'missing' }],
      [{ id: 'd1', title: 'OrphanDoc', slug: 'orphan-doc', folder_id: 'missing' }],
    );

    expect(tree.folders.map((f) => f.id)).toContain('f2');
    expect(tree.docs.map((d) => d.id)).toContain('d1');
  });

  it('preserves a null slug for unresolved docs', () => {
    const tree = buildNotesTree([], [{ id: 'd1', title: 'Untitled', slug: null, folder_id: null }]);

    expect(tree.docs[0]?.slug).toBeNull();
  });
});
