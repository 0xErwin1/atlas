<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AppRail from '@/components/shell/AppRail.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import GlobalDialogs from '@/components/shell/GlobalDialogs.vue';
import MobileTabBar from '@/components/shell/MobileTabBar.vue';
import Icon from '@/components/ui/Icon.vue';
import TabStrip from '@/components/ui/TabStrip.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { provideDocsShell } from '@/composables/useDocsShell';
import { useDocsTabs } from '@/composables/useDocsTabs';
import { formatShortcut } from '@/lib/keymap';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesSidebar from '@/views/NotesSidebar.vue';

// The persistent shell for the Docs group (notes, tasks, task views, task
// detail). It mounts the rail and the notes sidebar once and keeps them across
// navigation between those routes; only the nested <router-view> content swaps.
const route = useRoute();
const router = useRouter();
const ui = useUiStore();
const workspace = useWorkspaceStore();
const { isMobile } = useBreakpoint();

const sidebarRef = ref<InstanceType<typeof NotesSidebar> | null>(null);

// The tab strip "+" in the notes content still opens a new page in the (now
// hoisted) sidebar tree; the content pane reaches it through this bridge.
provideDocsShell({ openNewPage: () => sidebarRef.value?.openNewPage() });

// The unified tab strip for the Docs group. It lives here so it persists across
// the notes and tasks routes; only the nested content below it swaps.
const { tabs: docsTabs, onSelect, onClose, onCloseOthers, onCloseRight, onCloseAll } = useDocsTabs();
const commandPaletteShortcut = formatShortcut('command-palette');

function openNewPage(): void {
  sidebarRef.value?.openNewPage();
}

const activeWorkspace = computed(() =>
  workspace.workspaces.find((w) => w.slug === workspace.activeWorkspaceSlug),
);
const workspaceTitle = computed(
  () => activeWorkspace.value?.name ?? workspace.activeWorkspaceSlug ?? 'Atlas',
);

// On mobile a single pane shows at a time: the sidebar by default, or the routed
// content once the view has navigated into a detail. Tasks/task-detail are always
// detail; notes is a detail only when a document slug is open.
const mobileDetail = computed(() => {
  if (route.meta.mobileDetail === true) return true;
  if (route.name === 'notes') {
    const slug = route.params.slug;
    return typeof slug === 'string' && slug.length > 0;
  }
  return false;
});

function openSearch(): void {
  void router.push({ name: 'search' });
}
</script>

<template>
  <div
    v-if="isMobile"
    class="flex flex-col"
    style="height: 100dvh; overflow: hidden; background-color: var(--c-background);"
  >
    <main class="flex flex-col flex-1 min-w-0 overflow-hidden" style="background-color: var(--c-background);">
      <!-- The routed view stays mounted so it keeps loading; only its visibility
           follows the single-pane rule. -->
      <div v-show="mobileDetail" class="flex flex-col flex-1 min-h-0">
        <router-view />
      </div>

      <div v-show="!mobileDetail" class="flex flex-col flex-1 min-h-0">
        <div
          class="flex items-center"
          style="height: 44px; flex: 0 0 44px; padding: 0 12px; gap: 8px; border-bottom: 1px solid var(--c-border);"
        >
          <span
            class="flex-1 truncate"
            style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground);"
          >
            {{ workspaceTitle }}
          </span>
          <div class="flex items-center" style="gap: 2px;">
            <button type="button" class="atl-gbtn" title="Search" aria-label="Search" @click="openSearch">
              <Icon name="search" :size="14" />
            </button>
          </div>
        </div>
        <div class="flex-1 overflow-y-auto overflow-x-hidden">
          <NotesSidebar ref="sidebarRef" />
        </div>
      </div>
    </main>

    <MobileTabBar />
    <GlobalDialogs />
  </div>

  <div
    v-else
    class="flex"
    style="height: 100vh; overflow: hidden; background-color: var(--c-background);"
  >
    <AppRail />

    <ContextSidebar v-if="!ui.sidebarCollapsed">
      <template #header-actions>
        <button type="button" class="atl-gbtn" title="Search" aria-label="Search" @click="openSearch">
          <Icon name="search" :size="14" />
        </button>
        <button
          type="button"
          class="atl-gbtn"
          title="Collapse sidebar"
          aria-label="Collapse sidebar"
          @click="ui.toggleSidebar()"
        >
          <Icon name="panel-left" :size="13" />
        </button>
      </template>
      <NotesSidebar ref="sidebarRef" />
    </ContextSidebar>

    <div class="flex flex-col flex-1 min-w-0">
      <TabStrip
        v-if="docsTabs.length > 0"
        :tabs="docsTabs"
        closable
        @select="onSelect"
        @close="onClose"
        @close-others="onCloseOthers"
        @close-right="onCloseRight"
        @close-all="onCloseAll"
      >
        <template #right>
          <button type="button" class="atl-gbtn" title="New page" aria-label="New page" @click="openNewPage">
            <Icon name="plus" :size="13" />
          </button>
          <button
            type="button"
            class="atl-gbtn"
            :title="`Command palette ${commandPaletteShortcut}`"
            aria-label="Command palette"
            @click="ui.openPalette()"
          >
            <Icon name="command" :size="13" />
          </button>
        </template>
      </TabStrip>

      <div class="flex flex-1 min-h-0">
        <router-view />
      </div>
    </div>

    <GlobalDialogs />
  </div>
</template>
