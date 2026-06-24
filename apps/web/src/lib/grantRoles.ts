export type GrantRole = 'viewer' | 'editor' | 'admin';
export type PrincipalType = 'user' | 'api_key' | 'group';

const ALL_ROLES: GrantRole[] = ['viewer', 'editor', 'admin'];

/**
 * The highest role an agent (api_key) principal may ever hold. This is the E03
 * regression guard: agents are capped at editor and must never be granted admin
 * or the ability to manage grants. The cap is enforced both here (UI) and on the
 * server (authorize_share); the UI must never offer or send admin for an agent.
 */
export const MAX_AGENT_ROLE: GrantRole = 'editor';

const AGENT_ROLES: GrantRole[] = ['viewer', 'editor'];

/**
 * Whether the principal type carries the agent cap. Only api_key principals are
 * agents; users and groups (a set of users) are uncapped. Unknown types are
 * treated as agents (conservative — no admin).
 */
function isAgentPrincipal(principalType: string): boolean {
  return principalType !== 'user' && principalType !== 'group';
}

/**
 * Roles selectable for the given principal type. Users and groups get all three;
 * an agent (api_key / unknown) is capped at editor — admin is never offered.
 */
export function availableRolesFor(principalType: string): GrantRole[] {
  return isAgentPrincipal(principalType) ? [...AGENT_ROLES] : [...ALL_ROLES];
}

/**
 * Whether a principal of the given type may be granted the given role. The
 * security invariant: an agent can never be admin. Unknown principal types are
 * treated as agents (conservative — no admin).
 */
export function isRoleAllowedFor(principalType: string, role: GrantRole): boolean {
  return availableRolesFor(principalType).includes(role);
}
