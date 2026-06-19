<script setup lang="ts">
import { computed, useSlots } from 'vue';
import SettingsModal from '@/components/settings/SettingsModal.vue';
import ShareDialog from '@/components/share/ShareDialog.vue';
import AppRail from '@/components/shell/AppRail.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import InspectorDock from '@/components/shell/InspectorDock.vue';
import MobileTabBar from '@/components/shell/MobileTabBar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { type InspectorTab, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const INSPECTOR_TABS: Array<{ id: InspectorTab; label: string; icon: string }> = [
  { id: 'properties', label: 'Properties', icon: 'hash' },
  { id: 'backlinks', label: 'Backlinks', icon: 'link' },
  { id: 'activity', label: 'Activity', icon: 'clock' },
  { id: 'share', label: 'Share', icon: 'user' },
];

const props = withDefaults(
  defineProps<{
    sidebarTitle?: string;
    sidebarIcon?: string;
    // On mobile a single pane is shown at a time: the sidebar (list/tree) by
    // default, or the main content when the view has navigated into a detail.
    mobileDetail?: boolean;
  }>(),
  {
    sidebarTitle: 'Explorer',
    sidebarIcon: '',
    mobileDetail: false,
  },
);

const ui = useUiStore();
const workspace = useWorkspaceStore();
const slots = useSlots();
const { isMobile } = useBreakpoint();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// The inspector tabs are item-scoped (properties, backlinks, …). A view that
// provides no inspector slot — e.g. the board — has nothing to show there, so the
// dock is hidden rather than opening to a blank panel.
const hasInspector = computed(() => Object.keys(slots).some((name) => name.startsWith('inspector-')));

const hasSidebar = computed(() => Boolean(slots.sidebar));

// With no sidebar (e.g. the board) the main content is always the primary pane.
const showMainOnMobile = computed(() => props.mobileDetail || !hasSidebar.value);
</script>

<template>
  <div
    v-if="isMobile"
    class="flex flex-col"
    style="height: 100dvh; overflow: hidden; background-color: var(--c-background);"
  >
    <main class="flex flex-col flex-1 min-w-0 overflow-hidden" style="background-color: var(--c-background);">
      <template v-if="showMainOnMobile">
        <slot />
      </template>
      <template v-else>
        <div
          class="flex items-center"
          style="height: 44px; flex: 0 0 44px; padding: 0 12px; gap: 8px; border-bottom: 1px solid var(--c-border);"
        >
          <span
            class="flex-1 truncate"
            style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground);"
          >
            {{ sidebarTitle }}
          </span>
          <div class="flex items-center" style="gap: 2px;">
            <slot name="sidebar-actions" />
          </div>
        </div>
        <div class="flex-1 overflow-y-auto overflow-x-hidden">
          <slot name="sidebar" />
        </div>
        <div v-if="$slots['sidebar-footer']" style="border-top: 1px solid var(--c-border); padding: 8px;">
          <slot name="sidebar-footer" />
        </div>
      </template>
    </main>

    <MobileTabBar />

    <BottomSheet
      v-if="hasInspector"
      :open="ui.inspectorOpen"
      title="Details"
      @close="ui.toggleInspector()"
    >
      <div class="flex" style="gap: 2px; margin-bottom: 12px;">
        <button
          v-for="tab in INSPECTOR_TABS"
          :key="tab.id"
          type="button"
          class="flex items-center justify-center flex-1"
          :aria-selected="ui.inspectorTab === tab.id"
          :style="`
            gap: 6px;
            height: 34px;
            border: none;
            border-radius: var(--r-md);
            cursor: pointer;
            background: ${ui.inspectorTab === tab.id ? 'var(--c-selection)' : 'transparent'};
            color: ${ui.inspectorTab === tab.id ? 'var(--c-primary)' : 'var(--c-muted)'};
            font-size: var(--fs-sm);
          `"
          @click="ui.setInspectorTab(tab.id)"
        >
          <Icon :name="tab.icon" :size="15" />
        </button>
      </div>

      <slot :name="`inspector-${ui.inspectorTab}`" :tab="ui.inspectorTab">
        <EmptyState
          icon="panel-right"
          title="Nothing to show"
          hint="This panel has no content for the current view."
        />
      </slot>
    </BottomSheet>

    <BannerToast />

    <ShareDialog
      :open="ui.shareOpen"
      :ws="ws"
      :resource-label="ui.shareResourceLabel"
      @close="ui.closeShare()"
    />

    <SettingsModal />
  </div>

  <div
    v-else
    class="flex"
    style="height: 100vh; overflow: hidden; background-color: var(--c-background);"
  >
    <AppRail />

    <ContextSidebar v-if="!ui.sidebarCollapsed" :title="sidebarTitle" :icon="sidebarIcon">
      <template #header-actions>
        <slot name="sidebar-actions" />
      </template>
      <slot name="sidebar" />
      <template v-if="$slots['sidebar-footer']" #footer>
        <slot name="sidebar-footer" />
      </template>
    </ContextSidebar>

    <main
      class="flex flex-col flex-1 min-w-0 overflow-hidden"
      style="background-color: var(--c-background);"
    >
      <slot />
    </main>

    <InspectorDock v-if="hasInspector">
      <template #default="{ tab }">
        <slot :name="`inspector-${tab}`" :tab="tab">
          <EmptyState
            icon="panel-right"
            title="Nothing to show"
            hint="This panel has no content for the current view."
          />
        </slot>
      </template>
    </InspectorDock>

    <BannerToast />

    <ShareDialog
      :open="ui.shareOpen"
      :ws="ws"
      :resource-label="ui.shareResourceLabel"
      @close="ui.closeShare()"
    />

    <SettingsModal />
  </div>
</template>
