import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ColorPicker from '@/components/ui/ColorPicker.vue';

describe('ColorPicker', () => {
  it('emits the swatch id when a named swatch is clicked', async () => {
    const wrapper = mount(ColorPicker);
    const blue = wrapper.find('button.swatch[aria-label="Blue"]');

    await blue.trigger('click');

    expect(wrapper.emitted('select')?.[0]).toEqual(['blue']);
  });

  it('emits the hex string when a valid #RRGGBB is entered in the text field', async () => {
    const wrapper = mount(ColorPicker);
    const text = wrapper.find('input.hex-text');

    await text.setValue('#1A2B3C');

    expect(wrapper.emitted('select')?.at(-1)).toEqual(['#1A2B3C']);
  });

  it('does not emit while the typed hex is incomplete or malformed', async () => {
    const wrapper = mount(ColorPicker);
    const text = wrapper.find('input.hex-text');

    await text.setValue('#1A2');
    await text.setValue('#GGGGGG');

    expect(wrapper.emitted('select')).toBeUndefined();
  });

  it('emits the hex from the native color input', async () => {
    const wrapper = mount(ColorPicker);
    const native = wrapper.find('input[type="color"]');

    await native.setValue('#ff8800');

    expect(wrapper.emitted('select')?.at(-1)).toEqual(['#ff8800']);
  });

  it('highlights the named swatch matching the selection', () => {
    const wrapper = mount(ColorPicker, { props: { selected: 'green' } });
    expect(wrapper.find('button.swatch[aria-label="Green"]').classes()).toContain('on');
  });

  it('shows a custom hex selection in the text field without highlighting a swatch', () => {
    const wrapper = mount(ColorPicker, { props: { selected: '#123456' } });

    expect(wrapper.findAll('button.swatch.on')).toHaveLength(0);
    expect((wrapper.find('input.hex-text').element as HTMLInputElement).value).toBe('#123456');
  });
});
