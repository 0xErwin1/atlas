import { describe, expect, it } from 'vitest';
import { buildNotesTree, docKey, flattenVisible, folderAncestors, folderKey } from '@/lib/notesTree';

describe('folderAncestors', () => {
  const folders = [
    { id: 'root', name: 'Root', parent_folder_id: null },
    { id: 'mid', name: 'Mid', parent_folder_id: 'root' },
    { id: 'leaf', name: 'Leaf', parent_folder_id: 'mid' },
  ];

  it('returns the chain from the start folder up to the root', () => {
    expect(folderAncestors(folders, 'leaf')).toEqual(['leaf', 'mid', 'root']);
  });

  it('returns an empty array for a root-level document (no folder)', () => {
    expect(folderAncestors(folders, null)).toEqual([]);
  });

  it('stops safely on a cycle', () => {
    const cyclic = [
      { id: 'a', name: 'A', parent_folder_id: 'b' },
      { id: 'b', name: 'B', parent_folder_id: 'a' },
    ];
    expect(folderAncestors(cyclic, 'a')).toEqual(['a', 'b']);
  });
});

describe('flattenVisible', () => {
  const tree = buildNotesTree(
    [
      { id: 'f1', name: 'Alpha', parent_folder_id: null },
      { id: 'f1a', name: 'Inner', parent_folder_id: 'f1' },
    ],
    [
      { id: 'd1', title: 'In Alpha', slug: 'in-alpha', folder_id: 'f1' },
      { id: 'd2', title: 'Root doc', slug: 'root-doc', folder_id: null },
    ],
  );

  it('lists keys in render order when nothing is collapsed', () => {
    expect(flattenVisible(tree, () => false)).toEqual([
      folderKey('f1'),
      folderKey('f1a'),
      docKey('in-alpha'),
      docKey('root-doc'),
    ]);
  });

  it('omits the children of a collapsed folder', () => {
    expect(flattenVisible(tree, (id) => id === 'f1')).toEqual([folderKey('f1'), docKey('root-doc')]);
  });
});

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
