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

function sortTree(level: NotesTree): void {
  level.folders.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  level.docs.sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }));

  for (const folder of level.folders) {
    sortTree(folder);
  }
}
