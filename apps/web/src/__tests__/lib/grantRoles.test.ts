import { describe, expect, it } from 'vitest';
import { availableRolesFor, type GrantRole, isRoleAllowedFor, MAX_AGENT_ROLE } from '@/lib/grantRoles';

describe('grant role caps (REQ-W27 — E03 regression guard)', () => {
  it('offers viewer/editor/admin for a user principal', () => {
    expect(availableRolesFor('user')).toEqual<GrantRole[]>(['viewer', 'editor', 'admin']);
  });

  it('NEVER offers admin for an api_key (agent) principal — caps at editor', () => {
    const roles = availableRolesFor('api_key');

    expect(roles).toEqual<GrantRole[]>(['viewer', 'editor']);
    expect(roles).not.toContain('admin');
  });

  it('pins the agent cap to editor', () => {
    expect(MAX_AGENT_ROLE).toBe('editor');
  });

  it('rejects admin for an agent and accepts it for a user (the security invariant)', () => {
    expect(isRoleAllowedFor('api_key', 'admin')).toBe(false);
    expect(isRoleAllowedFor('api_key', 'editor')).toBe(true);
    expect(isRoleAllowedFor('api_key', 'viewer')).toBe(true);

    expect(isRoleAllowedFor('user', 'admin')).toBe(true);
    expect(isRoleAllowedFor('user', 'editor')).toBe(true);
    expect(isRoleAllowedFor('user', 'viewer')).toBe(true);
  });

  it('treats an unknown principal type conservatively (no admin)', () => {
    expect(isRoleAllowedFor('something-else', 'admin')).toBe(false);
    expect(availableRolesFor('something-else')).not.toContain('admin');
  });
});
