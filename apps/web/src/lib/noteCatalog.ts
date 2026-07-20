import { z } from 'zod';
import type { BoardSummaryDto } from '@/stores/boards';
import type { useDocumentsStore } from '@/stores/documents';
import type { useFoldersStore } from '@/stores/folders';

export type NoteCatalog = {
  folders: ReturnType<typeof useFoldersStore>['folders'];
  summaries: ReturnType<typeof useDocumentsStore>['summaries'];
  boards: BoardSummaryDto[];
};

/**
 * Validates the per-project note-tree catalog cached under the `note-tree:{ws}:{project}`
 * key (built and consumed by NotesSpace.vue). `boards` defaults to an empty array so a catalog
 * entry cached before boards were added to the schema still parses cleanly
 * instead of throwing and forcing a hard cache invalidation.
 */
export const noteCatalogSchema: z.ZodType<NoteCatalog> = z.object({
  folders: z.array(
    z
      .object({
        id: z.string(),
        name: z.string(),
        parent_folder_id: z.string().nullable().optional(),
        project_id: z.string().nullable().optional(),
        workspace_id: z.string(),
        created_at: z.string(),
        updated_at: z.string(),
      })
      .passthrough(),
  ),
  summaries: z.array(
    z
      .object({
        id: z.string(),
        slug: z.string().nullable().optional(),
        title: z.string(),
        folder_id: z.string().nullable().optional(),
        head_seq: z.number(),
        updated_at: z.string(),
      })
      .passthrough(),
  ),
  boards: z
    .array(
      z
        .object({
          id: z.string(),
          name: z.string(),
          folder_id: z.string().nullable().optional(),
          task_count: z.number(),
          created_at: z.string(),
          updated_at: z.string(),
        })
        .passthrough(),
    )
    .optional()
    .default([]),
}) as z.ZodType<NoteCatalog>;
