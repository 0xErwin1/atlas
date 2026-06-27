import type { DropdownOption } from '@/components/ui/Dropdown.vue';

/**
 * Membership roles a principal can hold inside a single workspace, ordered from
 * most to least privileged. Distinct from grant roles (`grantRoles.ts`), which
 * are the resource-sharing roles (viewer/editor/admin); this is the workspace
 * membership tier. Centralized here so labels, ordering, the owner gate, and the
 * badge styling stay consistent across every panel that renders or assigns them.
 */
export type WorkspaceRole = 'owner' | 'admin' | 'member';

const WORKSPACE_ROLE_LABELS: Record<WorkspaceRole, string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
};

const WORKSPACE_ROLE_ORDER: WorkspaceRole[] = ['owner', 'admin', 'member'];

const WORKSPACE_ROLE_TAG_CLASSES: Record<WorkspaceRole, string> = {
  owner: 'atl-role-owner',
  admin: 'atl-role-admin',
  member: 'atl-role-member',
};

export function isWorkspaceRole(value: string): value is WorkspaceRole {
  return value === 'owner' || value === 'admin' || value === 'member';
}

export function workspaceRoleLabel(role: string): string {
  return isWorkspaceRole(role) ? WORKSPACE_ROLE_LABELS[role] : role;
}

export function workspaceRoleTagClass(role: string): string {
  return isWorkspaceRole(role) ? WORKSPACE_ROLE_TAG_CLASSES[role] : WORKSPACE_ROLE_TAG_CLASSES.member;
}

/**
 * Coerce an arbitrary server string into a known role, defaulting to the least
 * privileged tier so an unrecognized value can never widen access in the UI.
 */
export function coerceWorkspaceRole(
  value: string | null | undefined,
  fallback: WorkspaceRole = 'member',
): WorkspaceRole {
  return value != null && isWorkspaceRole(value) ? value : fallback;
}

/**
 * Assignable membership roles as dropdown options, in privilege order. `owner`
 * is only included when `includeOwner` is true: only an owner (or a break-glass
 * global admin) may grant it, and the backend returns 403 otherwise, so the
 * option is hidden rather than offered to callers who cannot use it.
 */
export function workspaceRoleOptions(opts: { includeOwner?: boolean } = {}): DropdownOption[] {
  const includeOwner = opts.includeOwner ?? false;
  return WORKSPACE_ROLE_ORDER.filter((role) => includeOwner || role !== 'owner').map((value) => ({
    value,
    label: WORKSPACE_ROLE_LABELS[value],
  }));
}
