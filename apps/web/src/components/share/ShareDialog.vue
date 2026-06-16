<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import RoleMenu from '@/components/share/RoleMenu.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import Presence from '@/components/ui/Presence.vue';
import type { GrantRole } from '@/lib/grantRoles';
import { type GrantDto, useShareStore } from '@/stores/share';

export type Visibility = 'private' | 'workspace' | 'public';

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
  'update:visibility': [value: Visibility];
}>();

const share = useShareStore();

const openMenuFor = ref<string | null>(null);

watch(
  () => [props.open, props.ws] as const,
  ([open, ws]) => {
    if (open) {
      openMenuFor.value = null;
      void share.load(ws);
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
  { value: 'public', icon: 'globe', label: 'Public', desc: 'Anyone with the link can view' },
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

function chooseVisibility(value: Visibility) {
  emit('update:visibility', value);
}

const grants = computed(() => share.grants);
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
          class="inline-flex items-center justify-center cursor-pointer"
          style="width: 26px; height: 26px; border: none; background: transparent; color: var(--c-muted); border-radius: var(--r-md);"
          @click="emit('close')"
        >
          <Icon name="x" :size="16" />
        </button>
      </div>

      <div style="padding: 16px;">
        <div class="flex" style="gap: 8px; margin-bottom: 18px;">
          <div
            class="flex items-center flex-1"
            style="gap: 8px; height: 32px; padding: 0 10px; background-color: var(--c-input); border: 1px solid var(--c-border); border-radius: var(--r-md);"
          >
            <Icon name="user" :size="14" :style="{ color: 'var(--c-muted)' }" />
            <span style="font-size: var(--fs-base); color: var(--c-muted);">
              Add people or agents by name, email, or @handle
            </span>
          </div>
          <Btn variant="primary" style="height: 32px;">Invite</Btn>
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
          <Avatar :agent="isAgent(g)" :size="24" :name="principalLabel(g)" />
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
            <button
              type="button"
              :data-visibility="opt.value"
              :aria-pressed="visibility === opt.value"
              class="flex items-center w-full text-left cursor-pointer"
              :style="{
                gap: '10px',
                padding: '9px 11px',
                border: 'none',
                background: visibility === opt.value ? 'var(--c-selection)' : 'transparent',
                boxShadow: visibility === opt.value ? 'inset 2px 0 0 var(--c-primary)' : 'none',
              }"
              @click="chooseVisibility(opt.value)"
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
            </button>
          </template>
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
        <span class="inline-flex items-center" style="gap: 6px; font-size: var(--fs-sm); color: var(--c-muted);">
          <Presence :size="7" />
          Actor-aware
        </span>
        <Btn variant="primary" @click="emit('close')">Done</Btn>
      </div>
    </div>
  </div>
</template>
