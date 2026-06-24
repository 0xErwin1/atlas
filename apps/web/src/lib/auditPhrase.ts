/**
 * Render a security audit entry into a short, human-readable phrase.
 *
 * The `action` verb drives the sentence; `metadata` is read defensively (it is
 * untrusted JSON from the wire and its shape varies per verb) and only used to
 * enrich the phrase when a known, typed field is present. `targetLabel` is the
 * server-resolved name of the affected subject (user display_name / api key
 * name) and is woven in when available, falling back to a generic noun.
 *
 * Unknown verbs fall back to a humanised form of the action so the feed never
 * renders an empty or raw token.
 *
 * The returned phrase has NO actor prefix — callers compose
 * "{actor} {phrase}". It reads naturally after an actor name, e.g.
 * "changed Alice's role from member to admin".
 */

type Metadata = unknown;

function asRecord(metadata: Metadata): Record<string, unknown> | null {
  if (typeof metadata !== 'object' || metadata === null) return null;
  return metadata as Record<string, unknown>;
}

function stringField(metadata: Metadata, key: string): string | null {
  const record = asRecord(metadata);
  const value = record?.[key];
  return typeof value === 'string' && value.length > 0 ? value : null;
}

function boolField(metadata: Metadata, key: string): boolean | null {
  const record = asRecord(metadata);
  const value = record?.[key];
  return typeof value === 'boolean' ? value : null;
}

function humaniseAction(action: string): string {
  const spaced = action.replace(/[._]/g, ' ').trim();
  if (spaced === '') return 'performed a security action';
  return spaced;
}

function targetNoun(targetLabel: string | null | undefined, fallback: string): string {
  return targetLabel !== null && targetLabel !== undefined && targetLabel.length > 0 ? targetLabel : fallback;
}

function possessive(name: string): string {
  return name.endsWith('s') ? `${name}'` : `${name}'s`;
}

function membershipRoleChanged(metadata: Metadata, target: string): string {
  const oldRole = stringField(metadata, 'old_role');
  const newRole = stringField(metadata, 'new_role');
  if (oldRole !== null && newRole !== null) {
    return `changed ${possessive(target)} role from ${oldRole} to ${newRole}`;
  }
  if (newRole !== null) {
    return `changed ${possessive(target)} role to ${newRole}`;
  }
  return `changed ${possessive(target)} role`;
}

function grantCreated(metadata: Metadata, target: string): string {
  const resourceType = stringField(metadata, 'resource_type');
  const role = stringField(metadata, 'role');
  const granteeType = stringField(metadata, 'grantee_type');
  const subject = granteeType === 'api_key' ? `agent ${target}` : target;
  const scope = resourceType !== null ? ` on the ${resourceType}` : '';
  if (role !== null) {
    return `granted ${subject} ${role}${scope}`;
  }
  return `granted ${subject} access${scope}`;
}

function grantRevoked(metadata: Metadata, target: string): string {
  const resourceType = stringField(metadata, 'resource_type');
  const granteeType = stringField(metadata, 'grantee_type');
  const subject = granteeType === 'api_key' ? `agent ${target}` : target;
  const scope = resourceType !== null ? ` on the ${resourceType}` : '';
  return `revoked ${subject}'s access${scope}`;
}

function apiKeyCreated(metadata: Metadata, target: string): string {
  const name = stringField(metadata, 'key_name') ?? target;
  return `created the API key "${name}"`;
}

function userCreated(metadata: Metadata, target: string): string {
  const role = stringField(metadata, 'initial_role');
  if (role !== null) {
    return `created user ${target} as ${role}`;
  }
  return `created user ${target}`;
}

function systemAdminSet(metadata: Metadata, target: string): string {
  const isAdmin = boolField(metadata, 'is_system_admin');
  if (isAdmin === true) return `granted ${target} system admin`;
  if (isAdmin === false) return `revoked ${possessive(target)} system admin`;
  return `changed ${possessive(target)} system admin status`;
}

export function auditPhrase(action: string, metadata: Metadata, targetLabel?: string | null): string {
  const userTarget = targetNoun(targetLabel, 'a user');
  const keyTarget = targetNoun(targetLabel, 'an API key');

  switch (action) {
    case 'membership.role_changed':
      return membershipRoleChanged(metadata, userTarget);
    case 'membership.removed':
      return `removed ${userTarget} from the workspace`;
    case 'grant.created':
      return grantCreated(metadata, targetNoun(targetLabel, 'a member'));
    case 'grant.revoked':
      return grantRevoked(metadata, targetNoun(targetLabel, 'a member'));
    case 'api_key.created':
      return apiKeyCreated(metadata, keyTarget);
    case 'api_key.revoked':
      return `revoked the API key ${keyTarget === 'an API key' ? keyTarget : `"${keyTarget}"`}`;
    case 'api_key_grant.revoked':
      return `revoked ${possessive(keyTarget)} grant`;
    case 'user.created':
      return userCreated(metadata, userTarget);
    case 'user.disabled':
      return `disabled user ${userTarget}`;
    case 'user.enabled':
      return `enabled user ${userTarget}`;
    case 'user.system_admin_set':
      return systemAdminSet(metadata, userTarget);
    case 'user.password_reset':
      return `reset ${possessive(userTarget)} password`;
    case 'user.activation_regenerated':
      return `regenerated ${possessive(userTarget)} activation link`;
    case 'account.activated':
      return 'activated their account';
    default:
      return humaniseAction(action);
  }
}
