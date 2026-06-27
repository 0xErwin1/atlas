import { formatDate } from '@/lib/format';

/**
 * The AI assist actions offered on a task. Each one maps to a fixed instruction
 * appended to the task context when composing the hand-off prompt.
 */
export type AiAction = 'summarize' | 'subtasks' | 'similar' | 'start';

export interface AiActionMeta {
  value: AiAction;
  /** Menu / tab label. */
  label: string;
  /** Lucide icon name. */
  icon: string;
}

/**
 * The fields the prompt builder reads. A structural subset shared by the full
 * `TaskDto` (task detail) and the lighter `TaskSummaryDto` (board rows), so the
 * same builder serves the detail banner and the list context menu. Fields the
 * summary lacks (description, due date) are optional and simply omitted from the
 * prompt when absent.
 */
export interface AiPromptTask {
  readable_id: string;
  title: string;
  priority?: string | null;
  due_date?: string | null;
  estimate?: number | null;
  labels?: string[] | null;
  description?: string | null;
}

export const AI_ACTIONS: AiActionMeta[] = [
  { value: 'summarize', label: 'Summarize', icon: 'align-left' },
  { value: 'subtasks', label: 'Generate sub-tasks', icon: 'list-checks' },
  { value: 'similar', label: 'Find similar tasks', icon: 'search' },
  { value: 'start', label: 'Start this task', icon: 'play' },
];

const INSTRUCTIONS: Record<AiAction, string> = {
  summarize:
    'Summarize this task in a few concise bullet points, then suggest the single most important next step.',
  subtasks:
    'Break this task down into a checklist of concrete, independently actionable sub-tasks. Return them as a plain numbered list, each phrased as a short imperative.',
  similar:
    "Based on this task's title, description, and labels, describe what related or similar tasks I should look for, and suggest a short list of search keywords.",
  start:
    'Start working on this task now. Read the title and description, lay out a brief plan, then begin implementing it step by step. Ask me only for what you genuinely need before proceeding.',
};

export function aiActionLabel(action: AiAction): string {
  return AI_ACTIONS.find((a) => a.value === action)?.label ?? action;
}

/**
 * Compose a ready-to-paste prompt that embeds the task's details and the chosen
 * action's instruction. `extra` is the user's optional extra context; when blank
 * it is left out. The result is plain text meant to be copied into an AI agent.
 */
export function buildTaskAiPrompt(
  task: AiPromptTask,
  statusName: string | null,
  action: AiAction,
  extra = '',
): string {
  const header = `You are helping with the task ${task.readable_id}.`;

  const meta: string[] = [`Title: ${task.title}`];
  if (statusName !== null && statusName !== '') meta.push(`Status: ${statusName}`);
  if (task.priority != null && task.priority !== '') meta.push(`Priority: ${task.priority}`);
  if (task.due_date != null && task.due_date !== '') meta.push(`Due: ${formatDate(task.due_date)}`);
  if (task.estimate != null) meta.push(`Estimate: ${task.estimate} pts`);

  const labels = task.labels ?? [];
  if (labels.length > 0) meta.push(`Labels: ${labels.join(', ')}`);

  const description = (task.description ?? '').trim();
  const descriptionBlock = `Description:\n${description !== '' ? description : '(no description)'}`;

  const blocks = [header, meta.join('\n'), descriptionBlock, `Task: ${INSTRUCTIONS[action]}`];

  const trimmedExtra = extra.trim();
  if (trimmedExtra !== '') blocks.push(`Additional context from me:\n${trimmedExtra}`);

  return blocks.join('\n\n');
}
