import { describe, expect, it } from 'vitest';
import { auditPhrase } from '@/lib/auditPhrase';

describe('auditPhrase', () => {
  it('renders a membership role change with both roles and a possessive target', () => {
    const metadata = { old_role: 'member', new_role: 'admin' };
    expect(auditPhrase('membership.role_changed', metadata, 'Alice')).toBe(
      "changed Alice's role from member to admin",
    );
  });

  it('falls back to a generic role phrase when metadata lacks roles', () => {
    expect(auditPhrase('membership.role_changed', {}, 'Alice')).toBe("changed Alice's role");
    expect(auditPhrase('membership.role_changed', null, 'Alice')).toBe("changed Alice's role");
  });

  it('phrases a membership removal', () => {
    expect(auditPhrase('membership.removed', { role: 'member' }, 'Bob')).toBe(
      'removed Bob from the workspace',
    );
  });

  it('phrases a grant created, marking an agent grantee', () => {
    const metadata = { resource_type: 'project', role: 'editor', grantee_type: 'api_key' };
    expect(auditPhrase('grant.created', metadata, 'BotX')).toBe('granted agent BotX editor on the project');
  });

  it('phrases a grant created for a user grantee without the agent prefix', () => {
    const metadata = { resource_type: 'workspace', role: 'admin', grantee_type: 'user' };
    expect(auditPhrase('grant.created', metadata, 'Carol')).toBe('granted Carol admin on the workspace');
  });

  it('phrases a grant revoked', () => {
    const metadata = { resource_type: 'project', grantee_type: 'user' };
    expect(auditPhrase('grant.revoked', metadata, 'Dave')).toBe("revoked Dave's access on the project");
  });

  it('phrases an API key creation with the key name from metadata', () => {
    expect(auditPhrase('api_key.created', { key_name: 'ci-bot', key_type: 'bot' }, null)).toBe(
      'created the API key "ci-bot"',
    );
  });

  it('phrases an API key revocation using the target label', () => {
    expect(auditPhrase('api_key.revoked', { key_type: 'bot' }, 'ci-bot')).toBe(
      'revoked the API key "ci-bot"',
    );
  });

  it('phrases user lifecycle verbs', () => {
    expect(auditPhrase('user.created', { initial_role: 'member' }, 'Eve')).toBe('created user Eve as member');
    expect(auditPhrase('user.disabled', {}, 'Bob')).toBe('disabled user Bob');
    expect(auditPhrase('user.enabled', {}, 'Bob')).toBe('enabled user Bob');
    expect(auditPhrase('user.password_reset', {}, 'Bob')).toBe("reset Bob's password");
    expect(auditPhrase('user.activation_regenerated', {}, 'Bob')).toBe("regenerated Bob's activation link");
  });

  it('phrases system admin grant and revoke from the boolean metadata', () => {
    expect(auditPhrase('user.system_admin_set', { is_system_admin: true }, 'Faye')).toBe(
      'granted Faye system admin',
    );
    expect(auditPhrase('user.system_admin_set', { is_system_admin: false }, 'Faye')).toBe(
      "revoked Faye's system admin",
    );
  });

  it('phrases a self-service account activation', () => {
    expect(auditPhrase('account.activated', {}, 'Gus')).toBe('activated their account');
  });

  it('falls back to a humanised verb for unknown actions', () => {
    expect(auditPhrase('some.new_action', {}, null)).toBe('some new action');
    expect(auditPhrase('', null, null)).toBe('performed a security action');
  });

  it('uses a generic target noun when no label is provided', () => {
    expect(auditPhrase('user.disabled', {}, null)).toBe('disabled user a user');
    expect(auditPhrase('membership.removed', {}, undefined)).toBe('removed a user from the workspace');
  });

  it('ignores non-string metadata fields (defensive against untrusted input)', () => {
    const metadata = { old_role: 42, new_role: true };
    expect(auditPhrase('membership.role_changed', metadata, 'Alice')).toBe("changed Alice's role");
  });
});
