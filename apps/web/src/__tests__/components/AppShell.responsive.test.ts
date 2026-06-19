import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { h } from 'vue';

vi.mock('vue-router', () => ({
  useRoute: () => ({ name: 'notes' }),
  useRouter: () => ({ push: vi.fn() }),
}));

import { useUiStore } from '@/stores/ui';
import AppShell from '@/views/AppShell.vue';

function setViewportWidth(width: number): void {
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true, writable: true });
  window.dispatchEvent(new Event('resize'));
}

const stubs = {
  AppRail: { template: '<div data-stub="rail" />' },
  ContextSidebar: { template: '<aside data-stub="sidebar"><slot /></aside>' },
  InspectorDock: { template: '<aside data-stub="inspector" />' },
  MobileTabBar: { template: '<nav data-stub="tabbar" />' },
  ShareDialog: { template: '<div />' },
  SettingsModal: { template: '<div />' },
  BannerToast: { template: '<div />' },
  EmptyState: { template: '<div data-stub="empty" />' },
};

function mountShell(props: Record<string, unknown> = {}, withSidebar = true) {
  return mount(AppShell, {
    props,
    slots: {
      default: () => h('div', { 'data-test': 'main' }, 'MAIN'),
      ...(withSidebar ? { sidebar: () => h('div', { 'data-test': 'tree' }, 'TREE') } : {}),
    },
    global: { stubs },
  });
}

describe('AppShell responsive layout', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders the desktop rail and no tab bar above the mobile breakpoint', () => {
    setViewportWidth(1280);
    const wrapper = mountShell();

    expect(wrapper.find('[data-stub="rail"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="tabbar"]').exists()).toBe(false);
  });

  it('swaps to the bottom tab bar and drops desktop chrome on mobile', () => {
    setViewportWidth(390);
    const wrapper = mountShell();

    expect(wrapper.find('[data-stub="tabbar"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="rail"]').exists()).toBe(false);
    expect(wrapper.find('[data-stub="inspector"]').exists()).toBe(false);
  });

  it('shows the sidebar slot as the primary pane on mobile by default', () => {
    setViewportWidth(390);
    const wrapper = mountShell();

    expect(wrapper.find('[data-test="tree"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="main"]').exists()).toBe(false);
  });

  it('shows the main slot as the primary pane on mobile when mobileDetail is set', () => {
    setViewportWidth(390);
    const wrapper = mountShell({ mobileDetail: true });

    expect(wrapper.find('[data-test="main"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="tree"]').exists()).toBe(false);
  });

  it('shows the main slot on mobile when the view has no sidebar', () => {
    setViewportWidth(390);
    const wrapper = mountShell({}, false);

    expect(wrapper.find('[data-test="main"]').exists()).toBe(true);
  });

  it('keeps rendering both panes on desktop', () => {
    setViewportWidth(1280);
    const wrapper = mountShell();

    expect(wrapper.find('[data-test="main"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="tree"]').exists()).toBe(true);
  });

  function mountWithInspector() {
    return mount(AppShell, {
      slots: {
        default: () => h('div', { 'data-test': 'main' }, 'MAIN'),
        sidebar: () => h('div', { 'data-test': 'tree' }, 'TREE'),
        'inspector-properties': () => h('div', { 'data-test': 'props' }, 'PROPS'),
      },
      global: { stubs },
    });
  }

  it('surfaces the inspector as a Details bottom sheet on mobile when open', () => {
    setViewportWidth(390);
    const ui = useUiStore();
    ui.inspectorOpen = true;

    const wrapper = mountWithInspector();

    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
    expect(wrapper.text()).toContain('Details');
    expect(wrapper.find('[data-test="props"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="inspector"]').exists()).toBe(false);
  });

  it('does not show the Details sheet on mobile when the inspector is closed', () => {
    setViewportWidth(390);
    const ui = useUiStore();
    ui.inspectorOpen = false;

    const wrapper = mountWithInspector();

    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(wrapper.find('[data-test="props"]').exists()).toBe(false);
  });

  it('uses the desktop inspector dock above the breakpoint', () => {
    setViewportWidth(1280);
    const ui = useUiStore();
    ui.inspectorOpen = true;

    const wrapper = mountWithInspector();

    expect(wrapper.find('[data-stub="inspector"]').exists()).toBe(true);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });
});
