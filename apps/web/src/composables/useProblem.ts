import type { AtlasProblem } from '../api/problem';

export interface ProblemResult {
  message: string;
  hint: string | undefined;
  requestId: string | undefined;
}

export function useProblem(problem: AtlasProblem): ProblemResult {
  return {
    message: problem.title,
    hint: problem.hint,
    requestId: problem.request_id,
  };
}
