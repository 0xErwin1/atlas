import { describe, expect, it } from 'vitest';
import { AI_ACTIONS, type AiPromptTask, aiActionLabel, buildTaskAiPrompt } from '@/lib/aiPrompt';

const fullTask: AiPromptTask = {
  readable_id: 'ATL-42',
  title: 'Fix login 500s',
  priority: 'high',
  due_date: '2026-07-01T00:00:00Z',
  estimate: 3,
  labels: ['backend', 'auth'],
  description: 'Some users get a 500 on login.',
};

describe('buildTaskAiPrompt', () => {
  it('embeds the task context and the action instruction', () => {
    const prompt = buildTaskAiPrompt(fullTask, 'In progress', 'summarize');

    expect(prompt).toContain('the task ATL-42');
    expect(prompt).toContain('Title: Fix login 500s');
    expect(prompt).toContain('Status: In progress');
    expect(prompt).toContain('Priority: high');
    expect(prompt).toContain('Estimate: 3 pts');
    expect(prompt).toContain('Labels: backend, auth');
    expect(prompt).toContain('Description:\nSome users get a 500 on login.');
    expect(prompt).toContain('Task: Summarize this task');
  });

  it('switches the instruction per action', () => {
    expect(buildTaskAiPrompt(fullTask, null, 'subtasks')).toContain('Break this task down');
    expect(buildTaskAiPrompt(fullTask, null, 'similar')).toContain('similar tasks');
    expect(buildTaskAiPrompt(fullTask, null, 'start')).toContain('Start working on this task now');
  });

  it('omits absent fields and falls back for an empty description (list summary case)', () => {
    const summary: AiPromptTask = { readable_id: 'ATL-7', title: 'Bare task' };

    const prompt = buildTaskAiPrompt(summary, 'Todo', 'summarize');

    expect(prompt).toContain('Status: Todo');
    expect(prompt).not.toContain('Priority:');
    expect(prompt).not.toContain('Due:');
    expect(prompt).not.toContain('Estimate:');
    expect(prompt).not.toContain('Labels:');
    expect(prompt).toContain('Description:\n(no description)');
  });

  it('omits the status line when none is provided', () => {
    expect(buildTaskAiPrompt(fullTask, null, 'summarize')).not.toContain('Status:');
  });

  it('appends extra context only when non-blank', () => {
    expect(buildTaskAiPrompt(fullTask, null, 'summarize', '   ')).not.toContain('Additional context');

    const withExtra = buildTaskAiPrompt(fullTask, null, 'summarize', 'Focus on the token refresh path.');
    expect(withExtra).toContain('Additional context from me:\nFocus on the token refresh path.');
  });
});

describe('AI_ACTIONS', () => {
  it('exposes the four assist actions with labels and icons', () => {
    expect(AI_ACTIONS.map((a) => a.value)).toEqual(['summarize', 'subtasks', 'similar', 'start']);
    for (const action of AI_ACTIONS) {
      expect(action.label.length).toBeGreaterThan(0);
      expect(action.icon.length).toBeGreaterThan(0);
    }
  });

  it('resolves a label for an action value', () => {
    expect(aiActionLabel('start')).toBe('Start this task');
  });
});
