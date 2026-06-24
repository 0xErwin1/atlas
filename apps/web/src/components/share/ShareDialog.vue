<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import RoleMenu from '@/components/share/RoleMenu.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import type { GrantRole } from '@/lib/grantRoles';
import { useApiKeysStore } from '@/stores/apiKeys';
import { useGroupsStore } from '@/stores/groups';
import { type GrantDto, type PrincipalDto, type ShareResource, useShareStore } from '@/stores/share';

export type Visibility = 'private' | 'workspace' | 'public';

const props = withDefaults(
  defineProps<{
    open: boolean;
    ws: string;
    /** When set, the dialog manages project grants instead of workspace grants. */
    projectSlug?: string;
    resourceLabel?: string;
    visibility?: Visibility;
  }>(),
  {
    projectSlug: undefined,
    resourceLabel: '',
    visibility: 'workspace',
  },
);

const emit = defineEmits<{
  close: [];
}>();

const share = useShareStore();
const apiKeys = useApiKeysStore();
const groups = useGroupsStore();
const { isMobile } = useBreakpoint();

const memberQuery = ref('');
const linkCopied = ref(false);

const resource = computed<ShareResource>(() =>
  props.projectSlug !== undefined
    ? { kind: 'project', ws: props.ws, projectSlug: props.projectSlug }
    : { kind: 'workspace', ws: props.ws },
);

async function copyLink(): Promise<void> {
  try {
    await navigator.clipboard.writeText(window.location.href);
    linkCopied.value = true;
    setTimeout(() => {
      linkCopied.value = false;
    }, 1500);
  } catch {
    // clipboard unavailable (insecure context / denied)
  }
}

watch(
  () => [props.open, props.ws, props.projectSlug] as const,
  ([open, ,]) => {
    if (open) {
      memberQuery.value = '';
      void share.load(resource.value);
      void share.loadMembers(props.ws);
      void apiKeys.loadKeys();
      void groups.load(props.ws);
    }
  },
  { immediate: true },
);

const isGroup = (g: GrantDto) => g.principal.type === 'group';
// Only api_key (and unknown) principals are agents; groups are sets of users.
const isAgent = (g: GrantDto) => g.principal.type !== 'user' && g.principal.type !== 'group';

function roleLabel(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

/**
 * Resolved display name for a grant row. For api_key and group principals the id
 * is a UUID; we join client-side against the loaded keys, members, and groups
 * lists so the user sees a readable name rather than a bare UUID.
 */
function principalLabel(g: GrantDto): string {
  if (g.principal.type === 'user') return g.principal.id;

  if (g.principal.type === 'group') {
    const fromGroups = groups.groups.find((gr) => gr.id === g.principal.id);
    return fromGroups !== undefined ? fromGroups.name : g.principal.id;
  }

  const fromKeys = apiKeys.keys.find((k) => k.id === g.principal.id);
  if (fromKeys !== undefined) return fromKeys.name;

  const fromMembers = share.members.find((m) => m.id === g.principal.id);
  if (fromMembers !== undefined) return fromMembers.display;

  return g.principal.id;
}

function badgeFor(g: GrantDto): string | null {
  if (g.principal.type === 'group') return 'GROUP';
  if (g.principal.type === 'api_key') return 'SCRIPT';
  if (isAgent(g)) return 'AGENT';
  return null;
}

const VISIBILITY_OPTS: Array<{ value: Visibility; icon: string; label: string; desc: string }> = [
  { value: 'private', icon: 'lock', label: 'Private', desc: 'Only people invited above' },
  { value: 'workspace', icon: 'eye', label: 'Workspace', desc: 'Anyone in the Atlas workspace can view' },
  { value: 'public', icon: 'globe', label: 'Public', desc: 'Anyone with the link can view' },
];

async function onSelectRole(g: GrantDto, role: GrantRole) {
  await share.changeRole(resource.value, g.id, role);
}

async function onRemove(g: GrantDto) {
  await share.removeGrant(resource.value, g.id);
}

const grants = computed(() => share.grants);

const grantedIds = computed(() => new Set(share.grants.map((g) => g.principal.id)));

/**
 * Picker candidates: workspace members (users + already-granted keys) merged
 * with the caller's own api keys and the workspace's groups that don't yet have
 * a grant on this resource. De-duplicated by id.
 */
const allCandidates = computed<PrincipalDto[]>(() => {
  const seen = new Set<string>();
  const result: PrincipalDto[] = [];

  for (const m of share.members) {
    seen.add(m.id);
    result.push(m);
  }

  for (const k of apiKeys.keys) {
    if (!seen.has(k.id)) {
      result.push({
        id: k.id,
        display: k.name,
        principal_type: 'api_key',
        key_type: k.type,
      } satisfies PrincipalDto);
    }
  }

  for (const gr of groups.groups) {
    if (!seen.has(gr.id)) {
      result.push({
        id: gr.id,
        display: gr.name,
        principal_type: 'group',
      } satisfies PrincipalDto);
    }
  }

  return result;
});

const memberMatches = computed<PrincipalDto[]>(() => {
  const q = memberQuery.value.trim().toLowerCase();
  if (q === '') return [];

  return allCandidates.value.filter(
    (m) => !grantedIds.value.has(m.id) && m.display.toLowerCase().includes(q),
  );
});

async function selectMember(member: PrincipalDto): Promise<void> {
  const principal = { type: member.principal_type, id: member.id };
  const ok = await share.addGrant(resource.value, principal, 'viewer');

  if (ok) {
    memberQuery.value = '';
  }
}

const canInvite = computed(() => memberMatches.value.length > 0);

async function invite(): Promise<void> {
  const highlighted = memberMatches.value[0];
  if (highlighted === undefined) return;
  await selectMember(highlighted);
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 flex justify-center"
    :class="isMobile ? 'items-end' : 'items-center'"
    style="background-color: var(--c-overlay); z-index: 60;"
    @click.self="emit('close')"
  >
    <div
      role="dialog"
      aria-label="Share"
      :style="isMobile
        ? 'width: 100%; max-height: 90vh; overflow-y: auto; background-color: var(--c-panel); border-top: 1px solid var(--c-border); border-radius: var(--r-lg) var(--r-lg) 0 0; box-shadow: var(--shadow-lg);'
        : 'width: 580px; max-width: calc(100vw - 32px); background-color: var(--c-panel); border: 1px solid var(--c-border); border-radius: var(--r-lg); box-shadow: var(--shadow-lg); overflow: visible;'"
    >
      <div v-if="isMobile" class="flex justify-center" style="padding: 8px 0 0;" aria-hidden="true">
        <div style="width: 36px; height: 4px; border-radius: 9999px; background: var(--c-border);" />
      </div>

      <div
        class="flex items-center"
        style="gap: 10px; padding: 13px 16px; border-bottom: 1px solid var(--c-border);"
      >
        <Icon name="user" :size="18" :style="{ color: 'var(--c-foreground)' }" />
        <div class="flex-1 min-w-0">
          <div style="font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--c-foreground);">
            Share
          </div>
          <div style="font-size: var(--fs-sm); color: var(--c-muted); font-family: var(--font-mono);">
            {{ resourceLabel }}
          </div>
        </div>
        <button
          type="button"
          data-action="close"
          title="Close"
          aria-label="Close"
          class="atl-gbtn"
          style="width: 26px; height: 26px;"
          @click="emit('close')"
        >
          <Icon name="x" :size="16" />
        </button>
      </div>

      <div style="padding: 16px;">
        <div class="relative" style="margin-bottom: 18px;">
          <div class="flex items-center" style="gap: 8px;">
            <div
              class="flex flex-1 min-w-0 items-center"
              style="gap: 8px; height: 32px; padding: 0 10px; background-color: var(--c-input); border: 1px solid var(--c-border); border-radius: var(--r-md);"
            >
              <Icon name="user" :size="14" :style="{ color: 'var(--c-muted)' }" />
              <input
                v-model="memberQuery"
                type="text"
                data-member-search
                placeholder="Add people, groups, or agents by name, email, or @handle"
                autocomplete="off"
                class="flex-1 min-w-0"
                style="height: 100%; border: none; outline: none; background: transparent; font-size: var(--fs-base); color: var(--c-foreground);"
                @keydown.enter.prevent="invite"
              />
            </div>

            <span
              data-invite-role
              class="inline-flex items-center"
              style="gap: 5px; flex: 0 0 auto; font-size: var(--fs-sm); color: var(--c-foreground); border: 1px solid var(--c-border); border-radius: 2px; padding: 3px 8px; background-color: var(--c-secondary);"
            >
              Viewer
              <Icon name="chevron-down" :size="12" :style="{ color: 'var(--c-muted)' }" />
            </span>

            <Btn
              variant="primary"
              data-action="invite"
              :disabled="!canInvite"
              style="height: 32px; flex: 0 0 auto;"
              @click="invite"
            >
              Invite
            </Btn>
          </div>

          <div
            v-if="memberMatches.length > 0"
            role="listbox"
            data-member-results
            style="
              position: absolute;
              top: 36px;
              left: 0;
              right: 0;
              max-height: 220px;
              overflow-y: auto;
              background-color: var(--c-panel);
              border: 1px solid var(--c-border);
              border-radius: var(--r-lg);
              box-shadow: var(--shadow-lg);
              padding: 4px;
              z-index: 10;
            "
          >
            <button
              v-for="m in memberMatches"
              :key="m.id"
              type="button"
              role="option"
              data-member-option
              :data-principal-type="m.principal_type"
              class="atl-row flex items-center w-full text-left"
              style="
                gap: 10px;
                height: 34px;
                padding: 0 8px;
                border: none;
                border-radius: var(--r-md);
                background: transparent;
                cursor: pointer;
                color: var(--c-foreground);
              "
              @click="selectMember(m)"
            >
              <span
                v-if="m.principal_type === 'group'"
                class="atl-group-glyph inline-flex items-center justify-center shrink-0"
                style="width: 22px; height: 22px;"
                aria-hidden="true"
              >
                <Icon name="users" :size="13" />
              </span>
              <Avatar v-else :agent="m.principal_type !== 'user'" :size="22" :name="m.display" />
              <span class="flex-1 min-w-0 truncate" style="font-size: var(--fs-base); font-weight: var(--fw-medium);">
                {{ m.display }}
              </span>
              <AgentBadge
                v-if="m.principal_type === 'group'"
                label="GROUP"
              />
              <AgentBadge
                v-else-if="m.principal_type !== 'user'"
                :label="m.principal_type === 'api_key' ? 'SCRIPT' : 'AGENT'"
              />
            </button>
          </div>
        </div>

        <div
          v-if="share.error"
          data-share-error
          style="
            background-color: var(--c-banner-err-bg);
            border: 1px solid rgba(240, 113, 120, 0.5);
            border-radius: var(--r-md);
            padding: 8px 11px;
            margin-bottom: 14px;
            font-size: var(--fs-sm);
            color: var(--c-banner-err-fg);
          "
        >
          {{ share.error }}
        </div>

        <div
          style="font-size: 10px; font-weight: var(--fw-semibold); letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); margin-bottom: 4px;"
        >
          People, groups &amp; agents with access
        </div>

        <div
          v-for="g in grants"
          :key="g.id"
          data-grant-row
          :data-principal-type="g.principal.type"
          class="flex items-center relative"
          style="gap: 10px; padding: 8px 0;"
        >
          <span
            v-if="isGroup(g)"
            class="atl-group-glyph inline-flex items-center justify-center shrink-0"
            style="width: 24px; height: 24px;"
            aria-hidden="true"
          >
            <Icon name="users" :size="14" />
          </span>
          <Avatar v-else :agent="isAgent(g)" :size="24" :name="principalLabel(g)" />
          <div class="flex-1 min-w-0">
            <div
              class="flex items-center"
              style="gap: 6px; font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--c-foreground);"
            >
              <span class="truncate">{{ principalLabel(g) }}</span>
              <AgentBadge v-if="badgeFor(g)" :label="badgeFor(g) ?? 'AGENT'" />
            </div>
            <div style="font-size: var(--fs-sm); color: var(--c-muted); font-family: var(--font-mono);">
              {{ g.principal.id }}
            </div>
          </div>

          <Popover placement="bottom-end" width="200px">
            <template #trigger="{ toggle }">
              <button
                type="button"
                data-action="open-role-menu"
                class="inline-flex items-center cursor-pointer"
                style="gap: 5px; font-size: var(--fs-sm); color: var(--c-foreground); border: 1px solid var(--c-border); border-radius: var(--r-md); padding: 3px 8px; background-color: var(--c-secondary);"
                @click="toggle"
              >
                {{ roleLabel(g.role) }}
                <Icon name="chevron-down" :size="12" :style="{ color: 'var(--c-muted)' }" />
              </button>
            </template>
            <template #default="{ close }">
              <RoleMenu
                :principal-type="g.principal.type"
                :role="g.role"
                @select="(r) => { onSelectRole(g, r); close(); }"
                @remove="() => { onRemove(g); close(); }"
              />
            </template>
          </Popover>
        </div>

        <div style="height: 1px; background-color: var(--c-border); margin: 14px 0;" />

        <div
          style="font-size: 10px; font-weight: var(--fw-semibold); letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); margin-bottom: 6px;"
        >
          General access
        </div>

        <div style="border: 1px solid var(--c-border); border-radius: var(--r-md); overflow: hidden;">
          <template v-for="(opt, i) in VISIBILITY_OPTS" :key="opt.value">
            <div
              v-if="i > 0"
              style="height: 1px; background-color: var(--c-border);"
            />
            <div
              :data-visibility="opt.value"
              :aria-current="visibility === opt.value ? 'true' : undefined"
              class="flex items-center w-full text-left"
              :style="{
                gap: '10px',
                padding: '9px 11px',
                background: visibility === opt.value ? 'var(--c-selection)' : 'transparent',
                boxShadow: visibility === opt.value ? 'inset 2px 0 0 var(--c-primary)' : 'none',
              }"
            >
              <Icon
                :name="opt.icon"
                :size="15"
                :style="{ color: visibility === opt.value ? 'var(--c-primary)' : 'var(--c-muted)', flex: '0 0 auto' }"
              />
              <div>
                <div
                  :style="{
                    fontSize: 'var(--fs-base)',
                    fontWeight: visibility === opt.value ? 'var(--fw-semibold)' : 'var(--fw-medium)',
                    color: 'var(--c-foreground)',
                  }"
                >
                  {{ opt.label }}
                </div>
                <div style="font-size: var(--fs-xs); color: var(--c-muted);">{{ opt.desc }}</div>
              </div>
            </div>
          </template>
        </div>

        <div
          style="font-size: var(--fs-xs); color: var(--c-muted); margin-top: 6px; line-height: 1.4;"
        >
          General access reflects the current scope. Switching it from here isn't available yet.
        </div>
      </div>

      <div
        class="flex items-center"
        style="gap: 10px; padding: 12px 16px; border-top: 1px solid var(--c-border);"
      >
        <Btn variant="ghost" @click="copyLink">
          <Icon :name="linkCopied ? 'check' : 'link'" :size="14" />
          {{ linkCopied ? 'Copied' : 'Copy link' }}
        </Btn>
        <div class="flex-1" />
        <span
          class="inline-flex items-center"
          style="
            gap: 6px;
            font-size: var(--fs-xs);
            font-weight: var(--fw-medium);
            font-family: var(--font-mono);
            color: var(--c-agent);
            border: 1px solid var(--c-agent-border);
            background: var(--c-agent-bg);
            border-radius: var(--r-md);
            padding: 2px 8px;
          "
        >
          <span
            class="atl-pulse"
            style="width: 6px; height: 6px; border-radius: 9999px; background: var(--c-agent); flex: 0 0 auto;"
          />
          Actor-aware
        </span>
        <Btn variant="primary" @click="emit('close')">Done</Btn>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-group-glyph {
  border-radius: 2px;
  color: var(--c-primary);
  background: color-mix(in srgb, var(--c-primary) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--c-primary) 40%, transparent);
}
</style>
