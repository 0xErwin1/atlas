import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const componentSource = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

describe('truncated title contracts', () => {
  it('binds the complete title on every remaining confirmed task title surface', () => {
    const contracts = [
      [
        '../../components/tareas/TaskViewListView.vue',
        'class="atl-tl-title" :class="{ muted: isDone(task) }" :title="task.title"',
      ],
      ['../../components/tareas/TaskTableView.vue', 'class="atl-tt-title" :title="row.task.title"'],
      ['../../components/tareas/TaskTimelineView.vue', 'class="atl-tm-title" :title="bar.task.title"'],
    ] as const;

    for (const [path, binding] of contracts) {
      expect(componentSource(path)).toContain(binding);
    }
  });
});
