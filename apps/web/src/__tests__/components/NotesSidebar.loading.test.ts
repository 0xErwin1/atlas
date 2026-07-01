import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const source = readFileSync(resolve(process.cwd(), 'src/views/NotesSidebar.vue'), 'utf8');

describe('NotesSidebar loading state', () => {
  it('hides the notes tree until folders and documents finish loading', () => {
    expect(source).toContain('const treeLoading = computed(() => folders.loading || documents.loading);');
    expect(source).toContain('<LoadingState v-if="treeLoading" label="Loading notes…" />');
    expect(source).toContain('<NotesTree\n      v-else-if="activeProject"');
  });
});
