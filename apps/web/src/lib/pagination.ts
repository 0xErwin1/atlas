/** A cursor-paginated response envelope (`Page<T>`) from the Atlas API. */
export interface PageResult<T> {
  items: T[];
  next_cursor?: string | null;
  has_more?: boolean;
}

type FetchResult<T> = { data?: PageResult<T> | undefined; error?: unknown };

/**
 * Follows a cursor-paginated endpoint to completion, accumulating every page's
 * items. `fetchPage` receives the cursor for the next page (undefined for the
 * first) and returns the raw openapi-fetch `{ data, error }`. It stops at the
 * first error — returning what was collected so far plus the error — or when the
 * server reports no further pages.
 *
 * Use this for lists the UI must render in full (notes tree, kanban, pickers).
 * A single page would silently truncate them and, since items are ordered by
 * UUIDv7, hide the newest entries. Not for feeds that want recent-first +
 * load-more (e.g. search results, task activity).
 */
export async function collectPaged<T>(
  fetchPage: (cursor?: string) => Promise<FetchResult<T>>,
): Promise<{ items: T[]; error: unknown }> {
  const items: T[] = [];
  let cursor: string | undefined;

  for (;;) {
    const { data, error } = await fetchPage(cursor);

    if (error !== undefined || data === undefined) {
      return { items, error: error ?? new Error('Empty paginated response') };
    }

    items.push(...data.items);

    if (data.has_more !== true || data.next_cursor === null || data.next_cursor === undefined) {
      return { items, error: undefined };
    }
    cursor = data.next_cursor;
  }
}
