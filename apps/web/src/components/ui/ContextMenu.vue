<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';

/**
 * One menu entry. A plain entry renders as an action row; `{ sep: true }` renders
 * a divider; `{ header: true, label }` renders an uppercase group label. An entry
 * with `children` renders as a submenu parent: hovering it reveals a flyout with
 * the nested entries, and the entry's own `action` (if any) is ignored.
 */
export interface MenuItem {
  sep?: boolean;
  header?: boolean;
  label?: string;
  icon?: string;
  kbd?: string[];
  danger?: boolean;
  disabled?: boolean;
  action?: () => void;
  children?: MenuItem[];
}

const props = withDefaults(
  defineProps<{
    open: boolean;
    x: number;
    y: number;
    items: MenuItem[];
    width?: number;
  }>(),
  {
    width: 210,
  },
);

const emit = defineEmits<{
  close: [];
}>();

const openSub = ref<number | null>(null);

function hasChildren(item: MenuItem): boolean {
  return Array.isArray(item.children) && item.children.length > 0;
}

const left = computed(() => `${Math.max(8, Math.min(props.x, window.innerWidth - props.width - 8))}px`);

const top = computed(() => {
  const height = props.items.reduce((h, it) => h + (it.sep === true ? 9 : it.header === true ? 24 : 28), 8);
  return `${Math.max(8, Math.min(props.y, window.innerHeight - height - 8))}px`;
});

/** Flip the flyout to the left when there is no room for it on the right. */
const subOnLeft = computed(() => props.x + props.width * 2 + 16 > window.innerWidth);

function onMousedown(event: MouseEvent): void {
  const menu = document.getElementById('atl-context-menu');
  const target = event.target as Node | null;
  if (menu !== null && target !== null && !menu.contains(target)) {
    emit('close');
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') emit('close');
}

function run(item: MenuItem): void {
  if (item.disabled === true || hasChildren(item) || item.action === undefined) return;
  item.action();
  emit('close');
}

onMounted(() => {
  window.addEventListener('mousedown', onMousedown);
  window.addEventListener('keydown', onKeydown);
});

onUnmounted(() => {
  window.removeEventListener('mousedown', onMousedown);
  window.removeEventListener('keydown', onKeydown);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      id="atl-context-menu"
      class="atl-menu"
      role="menu"
      :style="{
        position: 'fixed',
        top,
        left,
        width: `${width}px`,
        background: 'var(--c-raised)',
        border: '1px solid var(--c-border)',
        borderRadius: '4px',
        boxShadow: 'var(--shadow-md)',
        padding: '4px 0',
        fontFamily: 'var(--font-ui)',
        zIndex: 200,
      }"
    >
      <template v-for="(item, i) in items" :key="i">
        <div
          v-if="item.sep"
          aria-hidden="true"
          style="height: 1px; background: var(--c-border); margin: 4px 0;"
        />
        <div
          v-else-if="item.header"
          style="font-size: 10px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); padding: 6px 10px 4px;"
        >
          {{ item.label }}
        </div>
        <div
          v-else-if="hasChildren(item)"
          class="atl-mi-wrap"
          @mouseenter="openSub = i"
          @mouseleave="openSub = null"
        >
          <div
            role="menuitem"
            aria-haspopup="true"
            class="atl-mi"
            :class="{ disabled: item.disabled === true }"
          >
            <span
              class="flex items-center"
              :style="{ width: '16px', flex: '0 0 auto', color: 'var(--c-muted)' }"
            >
              <Icon v-if="item.icon" :name="item.icon" :size="15" />
            </span>
            <span style="flex: 1;">{{ item.label }}</span>
            <span class="flex items-center" style="flex: 0 0 auto; color: var(--c-muted);">
              <Icon name="chevron-right" :size="14" />
            </span>
          </div>

          <div
            v-if="openSub === i"
            class="atl-submenu"
            role="menu"
            :style="{
              position: 'absolute',
              top: '-4px',
              [subOnLeft ? 'right' : 'left']: '100%',
              minWidth: `${width}px`,
              maxHeight: '60vh',
              overflowY: 'auto',
              background: 'var(--c-raised)',
              border: '1px solid var(--c-border)',
              borderRadius: '4px',
              boxShadow: 'var(--shadow-md)',
              padding: '4px 0',
              zIndex: 201,
            }"
          >
            <template v-for="(child, ci) in item.children" :key="ci">
              <div
                v-if="child.sep"
                aria-hidden="true"
                style="height: 1px; background: var(--c-border); margin: 4px 0;"
              />
              <div
                v-else-if="child.header"
                style="font-size: 10px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); padding: 6px 10px 4px;"
              >
                {{ child.label }}
              </div>
              <div
                v-else
                role="menuitem"
                class="atl-mi"
                :class="{ danger: child.danger === true, disabled: child.disabled === true }"
                @click="run(child)"
              >
                <span
                  class="flex items-center"
                  :style="{ width: '16px', flex: '0 0 auto', color: child.danger === true ? 'var(--c-danger)' : 'var(--c-muted)' }"
                >
                  <Icon v-if="child.icon" :name="child.icon" :size="15" />
                </span>
                <span style="flex: 1;">{{ child.label }}</span>
              </div>
            </template>
          </div>
        </div>
        <div
          v-else
          role="menuitem"
          class="atl-mi"
          :class="{ danger: item.danger === true, disabled: item.disabled === true }"
          @click="run(item)"
        >
          <span
            class="flex items-center"
            :style="{ width: '16px', flex: '0 0 auto', color: item.danger === true ? 'var(--c-danger)' : 'var(--c-muted)' }"
          >
            <Icon v-if="item.icon" :name="item.icon" :size="15" />
          </span>
          <span style="flex: 1;">{{ item.label }}</span>
          <span
            v-if="item.kbd && item.kbd.length"
            class="flex"
            style="gap: 3px; margin-left: 18px; flex: 0 0 auto;"
          >
            <span
              v-for="(k, ki) in item.kbd"
              :key="ki"
              style="min-width: 16px; padding: 0 4px; text-align: center; border: 1px solid var(--c-border); border-radius: var(--r-sm); background: var(--c-panel); color: var(--c-muted); font-family: var(--font-mono); font-size: 10px; line-height: 16px;"
            >
              {{ k }}
            </span>
          </span>
        </div>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-mi-wrap {
  position: relative;
}
</style>
