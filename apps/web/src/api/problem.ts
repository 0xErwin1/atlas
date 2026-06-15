export interface AtlasProblem {
  type: string;
  title: string;
  status: number;
  detail?: string;
  hint?: string;
  request_id?: string;
}

export interface ConflictProblem extends AtlasProblem {
  current_revision_id: string;
  current_seq: number;
  base_to_current_patch: string;
}

function isConflictProblem(p: AtlasProblem): p is ConflictProblem {
  return p.type.includes('revision-conflict');
}

export async function parseProblem(response: Response): Promise<AtlasProblem | ConflictProblem> {
  const raw: unknown = await response.json();

  if (
    typeof raw !== 'object' ||
    raw === null ||
    typeof (raw as Record<string, unknown>).type !== 'string' ||
    typeof (raw as Record<string, unknown>).title !== 'string' ||
    typeof (raw as Record<string, unknown>).status !== 'number'
  ) {
    return {
      type: 'urn:atlas:error:unknown',
      title: 'An unexpected error occurred',
      status: response.status,
    };
  }

  const problem = raw as AtlasProblem;

  if (isConflictProblem(problem)) {
    return problem as ConflictProblem;
  }

  return problem;
}
