import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const source = readFileSync(resolve(process.cwd(), 'src/components/tareas/TaskBody.vue'), 'utf8');

describe('TaskBody description editor', () => {
  it('does not clamp the editable description behind a show-more control', () => {
    expect(source).not.toContain('<CollapsibleText :collapsed-height="88">');
    expect(source).toContain('<TaskDescription :markdown="task.description"');
  });
});
