export interface FolderInput {
  id: string;
  name: string;
  parent_folder_id?: string | null;
}

export interface DocInput {
  id: string;
  title: string;
  slug?: string | null;
  folder_id?: string | null;
}

export interface BoardInput {
  id: string;
  name: string;
  folder_id?: string | null;
  task_count?: number;
}

/**
 * Returns the chain of folder ids from `startFolderId` up to the root, inclusive.
 * Used to reveal a document in the tree by expanding all of its ancestor folders.
 * Cycle-safe; stops at the first repeated id.
 */
export function folderAncestors(folders: FolderInput[], startFolderId: string | null): string[] {
  if (startFolderId === null) return [];

  const parentOf = new Map(folders.map((f) => [f.id, f.parent_folder_id ?? null]));

  const chain: string[] = [];
  const seen = new Set<string>();
  let current: string | null = startFolderId;

  while (current !== null && !seen.has(current)) {
    seen.add(current);
    chain.push(current);
    current = parentOf.get(current) ?? null;
  }

  return chain;
}

export interface TreeFolder {
  kind: 'folder';
  id: string;
  name: string;
  folders: TreeFolder[];
  docs: TreeDoc[];
  boards: TreeBoard[];
}

export interface TreeDoc {
  kind: 'doc';
  id: string;
  title: string;
  slug: string | null;
}

export interface TreeBoard {
  kind: 'board';
  id: string;
  name: string;
  taskCount: number;
}

export interface NotesTree {
  folders: TreeFolder[];
  docs: TreeDoc[];
  boards: TreeBoard[];
}

/**
 * Builds a folder/document/board tree from the flat folder, document, and board
 * lists returned by the project-scoped list endpoints (REQ-W14).
 *
 * Nesting is derived from `parent_folder_id` on folders and `folder_id` on
 * documents and boards. Items at the project root (no parent / no folder) sit
 * at the top level. A document, board, or folder whose parent is missing from
 * the input (e.g. filtered out by visibility) is reparented to the root so it
 * never disappears.
 *
 * Folders are sorted by name, documents by title, boards by name, all
 * case-insensitively, so the rendered tree is stable regardless of pagination
 * order.
 */
export function buildNotesTree(
  folders: FolderInput[],
  docs: DocInput[],
  boards: BoardInput[] = [],
): NotesTree {
  const nodes = new Map<string, TreeFolder>();

  for (const folder of folders) {
    nodes.set(folder.id, {
      kind: 'folder',
      id: folder.id,
      name: folder.name,
      folders: [],
      docs: [],
      boards: [],
    });
  }

  const root: NotesTree = { folders: [], docs: [], boards: [] };

  for (const folder of folders) {
    const node = nodes.get(folder.id);
    if (node === undefined) continue;

    const parentId = folder.parent_folder_id ?? null;
    const parent = parentId !== null ? nodes.get(parentId) : undefined;

    if (parent !== undefined) {
      parent.folders.push(node);
    } else {
      root.folders.push(node);
    }
  }

  for (const doc of docs) {
    const treeDoc: TreeDoc = {
      kind: 'doc',
      id: doc.id,
      title: doc.title,
      slug: doc.slug ?? null,
    };

    const folderId = doc.folder_id ?? null;
    const parent = folderId !== null ? nodes.get(folderId) : undefined;

    if (parent !== undefined) {
      parent.docs.push(treeDoc);
    } else {
      root.docs.push(treeDoc);
    }
  }

  for (const board of boards) {
    const treeBoard: TreeBoard = {
      kind: 'board',
      id: board.id,
      name: board.name,
      taskCount: board.task_count ?? 0,
    };

    const folderId = board.folder_id ?? null;
    const parent = folderId !== null ? nodes.get(folderId) : undefined;

    if (parent !== undefined) {
      parent.boards.push(treeBoard);
    } else {
      root.boards.push(treeBoard);
    }
  }

  sortTree(root);

  return root;
}

/** Stable selection key for a folder node. */
export function folderKey(id: string): string {
  return `folder:${id}`;
}

/** Stable selection key for a document node. */
export function docKey(slug: string): string {
  return `doc:${slug}`;
}

/** Stable selection key for a board node. */
export function boardKey(id: string): string {
  return `board:${id}`;
}

export interface TreeNodeRef {
  type: 'doc' | 'folder' | 'board';
  id: string;
}

/** Parses a selection/drag key (`folder:<id>` / `doc:<slug>` / `board:<id>`) back into a node ref. */
export function parseNodeKey(key: string): TreeNodeRef | null {
  const sep = key.indexOf(':');
  if (sep < 0) return null;
  const type = key.slice(0, sep);
  const id = key.slice(sep + 1);
  if (id === '') return null;
  if (type === 'doc' || type === 'folder' || type === 'board') return { type, id };
  return null;
}

/**
 * Flattens the tree into the ordered list of selectable node keys, in the exact
 * order they render, honouring the collapsed state. Used for shift-range
 * selection. Documents without a slug are not selectable and are omitted.
 *
 * Within a level, children render as folders, then documents, then boards.
 */
export function flattenVisible(tree: NotesTree, isCollapsed: (folderId: string) => boolean): string[] {
  const out: string[] = [];

  function walk(folders: TreeFolder[]): void {
    for (const folder of folders) {
      out.push(folderKey(folder.id));
      if (!isCollapsed(folder.id)) {
        walk(folder.folders);
        for (const doc of folder.docs) {
          if (doc.slug !== null) out.push(docKey(doc.slug));
        }
        for (const board of folder.boards) {
          out.push(boardKey(board.id));
        }
      }
    }
  }

  walk(tree.folders);
  for (const doc of tree.docs) {
    if (doc.slug !== null) out.push(docKey(doc.slug));
  }
  for (const board of tree.boards) {
    out.push(boardKey(board.id));
  }

  return out;
}

function sortTree(level: NotesTree): void {
  level.folders.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  level.docs.sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }));
  level.boards.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

  for (const folder of level.folders) {
    sortTree(folder);
  }
}
