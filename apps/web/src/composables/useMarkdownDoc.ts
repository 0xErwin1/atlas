import type { ConflictProblem } from '@/api/problem';
import { wrappedClient } from '@/api/wrapper';
import { joinFrontmatter, splitFrontmatter } from '@/lib/frontmatter';

export interface LoadResult {
  body: string;
  meta: Record<string, unknown>;
  headRevisionId: string;
}

export type SaveResult =
  | { kind: 'ok' }
  | { kind: 'conflict'; problem: ConflictProblem }
  | { kind: 'error'; hint: string | undefined; title: string };

/**
 * Composable that bridges the Atlas document API with the Tiptap editor.
 *
 * Responsibilities:
 * - Load a document and split frontmatter from the body before the editor
 *   receives it (REQ-W19).
 * - Save the document by joining frontmatter + body and performing a CAS PUT
 *   with the caller's base revision id (REQ-W15, W18).
 * - Return a typed result discriminating ok / conflict / error so the caller
 *   can delegate 3-way merge to useCasMerge (REQ-W18).
 *
 * This composable never performs the 3-way merge itself — that belongs to
 * useCasMerge. It signals conflicts back to the caller cleanly.
 */
export function useMarkdownDoc() {
  async function load(ws: string, slug: string): Promise<LoadResult> {
    const { data, error } = await wrappedClient.GET('/v1/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
    });

    if (error !== undefined || data === undefined) {
      throw new Error((error as { title?: string } | undefined)?.title ?? 'Failed to load document');
    }

    const raw = data.content ?? '';
    const { body, meta } = splitFrontmatter(raw);

    return {
      body,
      meta,
      headRevisionId: data.head_revision_id ?? '',
    };
  }

  async function save(
    ws: string,
    slug: string,
    body: string,
    meta: Record<string, unknown>,
    baseRevisionId: string,
  ): Promise<SaveResult> {
    const content = joinFrontmatter(meta, body);

    const { error } = await wrappedClient.PUT('/v1/workspaces/{ws}/documents/{slug}/content', {
      params: { path: { ws, slug } },
      body: { content, base_revision_id: baseRevisionId },
    });

    if (error === undefined) {
      return { kind: 'ok' };
    }

    const problem = error as Record<string, unknown>;
    const problemType = typeof problem.type === 'string' ? problem.type : '';

    if (problemType.includes('revision-conflict')) {
      return {
        kind: 'conflict',
        problem: problem as unknown as ConflictProblem,
      };
    }

    return {
      kind: 'error',
      hint: typeof problem.hint === 'string' ? problem.hint : undefined,
      title: typeof problem.title === 'string' ? problem.title : 'Save failed',
    };
  }

  return { load, save };
}
