/**
 * Extract a human-readable message from a rejected API call. The wrapped client
 * surfaces RFC 9457 problem responses, whose actionable `hint` is the message we
 * want to show; `title` is the next-best fallback, and `fallback` covers a
 * non-problem error (network failure, thrown string) where neither is present.
 *
 * Empty `hint`/`title` strings are treated as absent so a blank field never
 * shows as the message.
 */
export function errorHint(error: unknown, fallback: string): string {
  const problem = error as { hint?: string; title?: string } | undefined;

  const hint = problem?.hint;
  if (typeof hint === 'string' && hint.length > 0) return hint;

  const title = problem?.title;
  if (typeof title === 'string' && title.length > 0) return title;

  return fallback;
}
