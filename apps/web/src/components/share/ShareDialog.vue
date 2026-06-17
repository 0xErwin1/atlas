<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import RoleMenu from '@/components/share/RoleMenu.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import type { GrantRole } from '@/lib/grantRoles';
import { type GrantDto, type PrincipalDto, useShareStore } from '@/stores/share';

export type Visibility = 'private' | 'workspace';

const props = withDefaults(
  defineProps<{
    open: boolean;
    ws: string;
    resourceLabel?: string;
    visibility?: Visibility;
  }>(),
  {
    resourceLabel: '',
    visibility: 'workspace',
  },
);

const emit = defineEmits<{
  close: [];
}>();

const share = useShareStore();

const openMenuFor = ref<string | null>(null);
const memberQuery = ref('');

watch(
  () => [props.open, props.ws] as const,
  ([open, ws]) => {
    if (open) {
      openMenuFor.value = null;
      memberQuery.value = '';
      void share.load(ws);
      void share.loadMembers(ws);
    }
  },
  { immediate: true },
);

const isAgent = (g: GrantDto) => g.principal.type !== 'user';

function roleLabel(role: string): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function principalLabel(g: GrantDto): string {
  return isAgent(g) ? 'Agent' : g.principal.id;
}

function badgeFor(g: GrantDto): string | null {
  if (g.principal.type === 'api_key') return 'SCRIPT';
  if (isAgent(g)) return 'AGENT';
  return null;
}

const VISIBILITY_OPTS: Array<{ value: Visibility; icon: string; label: string; desc: string }> = [
  { value: 'private', icon: 'lock', label: 'Private', desc: 'Only people invited above' },
  { value: 'workspace', icon: 'eye', label: 'Workspace', desc: 'Anyone in the Atlas workspace can view' },
];

function toggleMenu(id: string) {
  openMenuFor.value = openMenuFor.value === id ? null : id;
}

async function onSelectRole(g: GrantDto, role: GrantRole) {
  openMenuFor.value = null;
  await share.changeRole(props.ws, g.id, role);
}

async function onRemove(g: GrantDto) {
  openMenuFor.value = null;
  await share.removeGrant(props.ws, g.id);
}

const grants = computed(() => share.grants);

const grantedIds = computed(() => new Set(share.grants.map((g) => g.principal.id)));

const memberMatches = computed<PrincipalDto[]>(() => {
  const q = memberQuery.value.trim().toLowerCase();
  if (q === '') return [];

  return share.members.filter((m) => !grantedIds.value.has(m.id) && m.display.toLowerCase().includes(q));
});

async function selectMember(member: PrincipalDto): Promise<void> {
  const principal = { type: member.principal_type, id: member.id };
  const ok = await share.addGrant(props.ws, principal, 'viewer');

  if (ok) {
    memberQuery.value = '';
  }
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 flex items-center justify-center"
    style="background-color: var(--c-overlay); z-index: 60;"
    @click.self="emit('close')"
  >
    <div
      role="dialog"
      aria-label="Share"
      style="
        width: 580px;
        max-width: calc(100vw - 32px);
        background-color: var(--c-panel);
        border: 1px solid var(--c-border);
        border-radius: var(--r-lg);
        box-shadow: var(--shadow-lg);
        overflow: visible;
      "
    >
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
          <div
            class="flex items-center"
            style="gap: 8px; height: 32px; padding: 0 10px; background-color: var(--c-input); border: 1px solid var(--c-border); border-radius: var(--r-md);"
          >
            <Icon name="user" :size="14" :style="{ color: 'var(--c-muted)' }" />
            <input
              v-model="memberQuery"
              type="text"
              data-member-search
              placeholder="Add people or agents by name"
              autocomplete="off"
              class="flex-1 min-w-0"
              style="height: 100%; border: none; outline: none; background: transparent; font-size: var(--fs-base); color: var(--c-foreground);"
            />
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
              <Avatar :agent="m.principal_type !== 'user'" :size="22" :name="m.display">
                <Icon
                  v-if="m.principal_type !== 'user'"
                  name="sparkles"
                  :size="11"
                />
              </Avatar>
              <span class="flex-1 min-w-0 truncate" style="font-size: var(--fs-base); font-weight: var(--fw-medium);">
                {{ m.display }}
              </span>
              <AgentBadge
                v-if="m.principal_type !== 'user'"
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
          People &amp; agents with access
        </div>

        <div
          v-for="g in grants"
          :key="g.id"
          data-grant-row
          :data-principal-type="g.principal.type"
          class="flex items-center relative"
          style="gap: 10px; padding: 8px 0;"
        >
          <Avatar :agent="isAgent(g)" :size="24" :name="principalLabel(g)">
            <Icon v-if="isAgent(g)" name="sparkles" :size="13" />
          </Avatar>
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

          <button
            type="button"
            data-action="open-role-menu"
            class="inline-flex items-center cursor-pointer"
            style="gap: 5px; font-size: var(--fs-sm); color: var(--c-foreground); border: 1px solid var(--c-border); border-radius: var(--r-md); padding: 3px 8px; background-color: var(--c-secondary);"
            @click="toggleMenu(g.id)"
          >
            {{ roleLabel(g.role) }}
            <Icon name="chevron-down" :size="12" :style="{ color: 'var(--c-muted)' }" />
          </button>

          <RoleMenu
            v-if="openMenuFor === g.id"
            :principal-type="g.principal.type"
            :role="g.role"
            @select="(r) => onSelectRole(g, r)"
            @remove="() => onRemove(g)"
          />
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
          Changing visibility is coming in a future release.
        </div>
      </div>

      <div
        class="flex items-center"
        style="gap: 10px; padding: 12px 16px; border-top: 1px solid var(--c-border);"
      >
        <Btn variant="ghost">
          <Icon name="link" :size="14" />
          Copy link
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
