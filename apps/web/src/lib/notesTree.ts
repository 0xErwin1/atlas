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

export interface TreeFolder {
  kind: 'folder';
  id: string;
  name: string;
  folders: TreeFolder[];
  docs: TreeDoc[];
}

export interface TreeDoc {
  kind: 'doc';
  id: string;
  title: string;
  slug: string | null;
}

export interface NotesTree {
  folders: TreeFolder[];
  docs: TreeDoc[];
}

/**
 * Builds a folder/document tree from the flat folder and document lists returned
 * by the project-scoped list endpoints (REQ-W14).
 *
 * Nesting is derived from `parent_folder_id` on folders and `folder_id` on
 * documents. Items at the project root (no parent / no folder) sit at the top
 * level. A document or folder whose parent is missing from the input (e.g.
 * filtered out by visibility) is reparented to the root so it never disappears.
 *
 * Folders are sorted by name, documents by title, both case-insensitively, so
 * the rendered tree is stable regardless of pagination order.
 */
export function buildNotesTree(folders: FolderInput[], docs: DocInput[]): NotesTree {
  const nodes = new Map<string, TreeFolder>();

  for (const folder of folders) {
    nodes.set(folder.id, {
      kind: 'folder',
      id: folder.id,
      name: folder.name,
      folders: [],
      docs: [],
    });
  }

  const root: NotesTree = { folders: [], docs: [] };

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

export interface TreeNodeRef {
  type: 'doc' | 'folder';
  id: string;
}

/** Parses a selection/drag key (`folder:<id>` / `doc:<slug>`) back into a node ref. */
export function parseNodeKey(key: string): TreeNodeRef | null {
  const sep = key.indexOf(':');
  if (sep < 0) return null;
  const type = key.slice(0, sep);
  const id = key.slice(sep + 1);
  if (id === '') return null;
  if (type === 'doc' || type === 'folder') return { type, id };
  return null;
}

/**
 * Flattens the tree into the ordered list of selectable node keys, in the exact
 * order they render, honouring the collapsed state. Used for shift-range
 * selection. Documents without a slug are not selectable and are omitted.
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
      }
    }
  }

  walk(tree.folders);
  for (const doc of tree.docs) {
    if (doc.slug !== null) out.push(docKey(doc.slug));
  }

  return out;
}

function sortTree(level: NotesTree): void {
  level.folders.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  level.docs.sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }));

  for (const folder of level.folders) {
    sortTree(folder);
  }
}
