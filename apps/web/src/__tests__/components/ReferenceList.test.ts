import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import type { ReferenceDto, TaskBacklinkDto } from '@/stores/taskDetail';

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
    origins: ['manual'],
    wikilink_reference_id: null,
    manual_reference_id: 'r1',
    manual_kind: 'blocks',
    manual_created_at: '2026-01-01T00:00:00Z',
    manual_created_by: { id: 'u1', type: 'user', display_name: 'U' },
    target_task_id: null,
    target_readable_id: null,
    target_document_id: null,
    target_title: null,
    target_resolved: true,
    ...overrides,
  };
}

function mountList(references: ReferenceDto[], backlinks: TaskBacklinkDto[] = []) {
  return mount(ReferenceList, {
    props: { references, backlinks },
    global: {
      plugins: [router],
      stubs: { Icon: true, Chip: { template: '<span class="chip"><slot /></span>' } },
    },
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
      reference({ manual_kind: 'docs', target_document_id: 'doc-9', target_title: 'Spec' }),
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

  it('lists inbound backlinks under "Referenced by", each linking to its source task', () => {
    const backlink: TaskBacklinkDto = {
      source_task_id: 't7',
      source_readable_id: 'ATL-7',
      source_title: 'Blocks this one',
      kind: 'blocks',
    };

    const wrapper = mountList([], [backlink]);

    expect(wrapper.text()).toContain('Referenced by');
    const row = wrapper.get('[data-backlink-id="t7"]');
    expect(row.text()).toContain('ATL-7');
    expect(row.text()).toContain('Blocks this one');
    expect(row.get('a.atl-ref-target').attributes('href')).toBe('/t/task/ATL-7');
  });

  it('labels merged and broken wikilinks and emits only an actionable manual reference id', async () => {
    const wrapper = mountList([
      reference({
        id: 'manual-1',
        origins: ['manual', 'wikilink'],
        wikilink_reference_id: 'link-1',
        manual_reference_id: 'manual-1',
        manual_kind: 'docs',
        manual_created_at: '2026-01-01T00:00:00Z',
        manual_created_by: { type: 'user', id: 'u1' },
        target_document_id: 'doc-1',
        target_title: 'Merged document',
      }),
      reference({
        id: 'link-2',
        origins: ['wikilink'],
        wikilink_reference_id: 'link-2',
        manual_reference_id: null,
        manual_kind: null,
        manual_created_at: null,
        manual_created_by: null,
        target_resolved: false,
        target_title: 'Missing document',
      }),
    ]);

    expect(wrapper.text()).toContain('manual');
    expect(wrapper.text()).toContain('wikilink');
    expect(wrapper.text()).toContain('broken');
    expect(wrapper.findAll('button')).toHaveLength(1);

    await wrapper.get('button').trigger('click');
    expect(wrapper.emitted('remove')).toEqual([['manual-1']]);
  });

  it('shows the empty state only when there are neither references nor backlinks', () => {
    expect(mountList([]).text()).toContain('No references.');
    expect(
      mountList(
        [],
        [{ source_task_id: 't7', source_readable_id: 'ATL-7', source_title: 'X', kind: 'relates' }],
      ).text(),
    ).not.toContain('No references.');
  });
});
