import { describe, expect, it } from 'vitest';
import { createNoteResourceState, runNoteResourceLoad } from '@/views/Notes.vue';

describe('NotesSidebar loading state', () => {
  it('keeps usable data visible during a same-target refresh', async () => {
    const state = createNoteResourceState();
    const target = { workspaceSlug: 'atlas', slug: 'cached-note' };

    await runNoteResourceLoad(state, target, async () => 'Cached note');
    const refresh = runNoteResourceLoad(state, target, async () => 'Fresh note');

    expect(state).toMatchObject({ target, status: 'pending', hasContent: true });
    await refresh;
    expect(state).toMatchObject({ target, status: 'ready', hasContent: true });
  });
});
