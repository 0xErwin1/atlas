import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/stores/share', () => ({
  useShareStore: () => ({
    grants: [],
    members: [],
    error: null,
    load: vi.fn(),
    loadMembers: vi.fn(),
    addGrant: vi.fn(),
    changeRole: vi.fn(),
    removeGrant: vi.fn(),
  }),
}));

vi.mock('@/stores/apiKeys', () => ({
  useApiKeysStore: () => ({
    keys: [],
    loading: false,
    error: null,
    loadKeys: vi.fn(),
  }),
}));

import ShareDialog from '@/components/share/ShareDialog.vue';

function setViewportWidth(width: number): void {
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true, writable: true });
  window.dispatchEvent(new Event('resize'));
}

function mountDialog() {
  return mount(ShareDialog, {
    props: { open: true, ws: 'atlas', resourceLabel: 'Doc · note' },
  });
}

describe('ShareDialog responsive', () => {
  it('centers the dialog on desktop', () => {
    setViewportWidth(1280);
    const wrapper = mountDialog();

    const overlay = wrapper.find('.fixed.inset-0');
    expect(overlay.classes()).toContain('items-center');
    expect(overlay.classes()).not.toContain('items-end');
  });

  it('anchors the dialog to the bottom as a sheet on mobile', () => {
    setViewportWidth(390);
    const wrapper = mountDialog();

    const overlay = wrapper.find('.fixed.inset-0');
    expect(overlay.classes()).toContain('items-end');
    expect(overlay.classes()).not.toContain('items-center');
  });
});
