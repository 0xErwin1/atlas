/**
 * Display helpers for an actor (a workspace member or an API-key agent), shared by
 * the activity feed and the comment threads so an agent principal renders
 * identically everywhere. An `api_key` principal is an agent; anything else is a
 * user, and a missing display name falls back to a generic label per kind.
 */
export function isAgent(actorType: string): boolean {
  return actorType === 'api_key';
}

export function actorName(displayName: string | null | undefined, actorType: string): string {
  return displayName ?? (isAgent(actorType) ? 'Agent' : 'User');
}
