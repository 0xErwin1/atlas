import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import type { ReferenceDto } from '@/stores/taskDetail';

const Dummy = { template: '<div />' };

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/t/task/:readableId', name: 'task-detail', component: Dummy },
    { path: '/n/:slug?', name: 'notes', component: Dummy },
  ],
});

function reference(overrides: Partial<ReferenceDto>): ReferenceDto {
  return {
    id: 'r1',
    kind: 'blocks',
    target_task_id: null,
    target_readable_id: null,
    target_document_id: null,
    target_title: null,
    target_resolved: true,
    created_at: '2026-01-01T00:00:00Z',
    created_by: { type: 'user', display_name: 'U' },
    ...overrides,
  } as ReferenceDto;
}

function mountList(references: ReferenceDto[]) {
  return mount(ReferenceList, {
    props: { references },
    global: { plugins: [router], stubs: { Icon: true, Chip: true } },
  });
}

describe('ReferenceList', () => {
  it('links a resolved task reference to its task detail route', () => {
    const wrapper = mountList([reference({ target_readable_id: 'ATL-2', target_task_id: 't2' })]);

    const link = wrapper.get('a.atl-ref-target');
    expect(link.attributes('href')).toBe('/t/task/ATL-2');
    expect(link.text()).toContain('ATL-2');
  });

  it('links a resolved document reference to the notes route by id', () => {
    const wrapper = mountList([
      reference({ kind: 'docs', target_document_id: 'doc-9', target_title: 'Spec' }),
    ]);

    expect(wrapper.get('a.atl-ref-target').attributes('href')).toBe('/n/doc-9');
  });

  it('renders a broken reference as plain text with no navigation', () => {
    const wrapper = mountList([
      reference({ target_readable_id: 'ATL-9', target_task_id: 't9', target_resolved: false }),
    ]);

    expect(wrapper.find('a.atl-ref-target').exists()).toBe(false);
    const span = wrapper.get('span.atl-ref-target');
    expect(span.text()).toContain('ATL-9');
    expect(span.attributes('style')).toContain('line-through');
  });
});
