import { z } from 'zod';
import type { AtlasProblem, ConflictProblem } from '@/api/problem';
import { isConflictProblem } from '@/api/problem';
import { wrappedClient } from '@/api/wrapper';
import { getResourceCachePrincipal, resourceCache } from '@/cache/cacheRuntime';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import { joinFrontmatter, splitFrontmatter } from '@/lib/frontmatter';

export interface LoadResult {
  /** The document's stable id, used to scope realtime reconcile to the open note. */
  id: string;
  body: string;
  meta: Record<string, unknown>;
  headRevisionId: string;
  /** The document's canonical slug, used to canonicalize a uuid-addressed URL. */
  slug: string | null;
}

export type SaveResult =
  | { kind: 'ok'; headRevisionId: string }
  | { kind: 'conflict'; problem: ConflictProblem }
  | { kind: 'error'; hint: string | undefined; title: string };

export interface MarkdownDocCacheOptions {
  workspaceId: string;
  onCached(document: LoadResult): void;
  isCurrent?(): boolean;
}

const loadResultSchema = z.object({
  id: z.string(),
  body: z.string(),
  meta: z.record(z.string(), z.unknown()),
  headRevisionId: z.string(),
  slug: z.string().nullable(),
});

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
  async function loadFromNetwork(ws: string, slug: string): Promise<LoadResult> {
    const { data, error, response } = await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
    });

    if (error !== undefined || data === undefined) {
      const err = new Error(
        (error as { title?: string } | undefined)?.title ?? 'Failed to load document',
      ) as Error & { status?: number };
      // The HTTP status is the reliable signal (the RFC 9457 body may omit it);
      // callers key their 404 recovery off it.
      err.status = response?.status ?? (error as { status?: number } | undefined)?.status ?? 0;
      throw err;
    }

    const raw = data.content ?? '';
    const { body, meta } = splitFrontmatter(raw);

    return {
      id: data.id ?? '',
      body,
      meta,
      headRevisionId: data.head_revision_id ?? '',
      slug: data.slug ?? null,
    };
  }

  async function load(ws: string, slug: string, cache?: MarkdownDocCacheOptions): Promise<LoadResult> {
    const key =
      cache === undefined
        ? null
        : buildCacheKey({
            principal: getResourceCachePrincipal(),
            workspaceId: cache.workspaceId,
            resourceKind: 'note-body',
            resourceId: slug,
          });

    if (key === null || cache === undefined) return loadFromNetwork(ws, slug);

    const isCurrent = cache.isCurrent ?? (() => true);
    const request = {
      key,
      payloadSchema: loadResultSchema,
      tags: [`document:${slug}`],
      freshForMs: CACHE_CADENCE.primary.freshForMs,
      activeForMs: CACHE_CADENCE.primary.activeForMs,
      retentionForMs: 24 * 60 * 60 * 1000,
      load: () => loadFromNetwork(ws, slug),
      publish: () => {},
      isCurrent,
    };

    await resourceCache.hydrate({ ...request, publish: cache.onCached });
    let result: LoadResult | undefined;
    const refresh = {
      ...request,
      publish: cache.onCached,
      load: async () => {
        result = await loadFromNetwork(ws, slug);
        return result;
      },
    };

    resourceCache.activate(refresh);
    await resourceCache.revalidate(refresh);

    if (result === undefined) return loadFromNetwork(ws, slug);
    return result;
  }

  async function save(
    ws: string,
    slug: string,
    body: string,
    meta: Record<string, unknown>,
    baseRevisionId: string,
  ): Promise<SaveResult> {
    const content = joinFrontmatter(meta, body);

    const { data, error } = await wrappedClient.PUT('/api/workspaces/{ws}/documents/{slug}/content', {
      params: { path: { ws, slug } },
      body: { content, base_revision_id: baseRevisionId },
    });

    if (error === undefined) {
      return { kind: 'ok', headRevisionId: data?.head_revision_id ?? '' };
    }

    const raw = error as Record<string, unknown>;
    const problem: AtlasProblem = {
      type: typeof raw.type === 'string' ? raw.type : 'urn:atlas:error:unknown',
      title: typeof raw.title === 'string' ? raw.title : 'Save failed',
      status: typeof raw.status === 'number' ? raw.status : 0,
      detail: typeof raw.detail === 'string' ? raw.detail : undefined,
      hint: typeof raw.hint === 'string' ? raw.hint : undefined,
      request_id: typeof raw.request_id === 'string' ? raw.request_id : undefined,
    };

    if (isConflictProblem(problem)) {
      const conflictProblem: ConflictProblem = {
        ...problem,
        current_revision_id: typeof raw.current_revision_id === 'string' ? raw.current_revision_id : '',
        current_seq: typeof raw.current_seq === 'number' ? raw.current_seq : 0,
        base_to_current_patch: typeof raw.base_to_current_patch === 'string' ? raw.base_to_current_patch : '',
      };
      return {
        kind: 'conflict',
        problem: conflictProblem,
      };
    }

    return {
      kind: 'error',
      hint: problem.hint,
      title: problem.title,
    };
  }

  return { load, save };
}
