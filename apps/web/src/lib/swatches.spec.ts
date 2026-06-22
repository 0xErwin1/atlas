import { describe, expect, it } from 'vitest';
import { swatchById } from '@/lib/swatches';

describe('swatchById', () => {
  it('resolves a named swatch unchanged', () => {
    const blue = swatchById('blue');
    expect(blue.id).toBe('blue');
    expect(blue.fg).toBe('var(--c-info)');
    expect(blue.bg).toBe('rgba(89, 194, 255, 0.12)');
    expect(blue.border).toBe('rgba(89, 194, 255, 0.4)');
  });

  it('falls back to neutral for an unknown id', () => {
    expect(swatchById('not-a-color').id).toBe('neutral');
    expect(swatchById(undefined).id).toBe('neutral');
  });

  it('synthesizes a swatch from a valid #RRGGBB hex', () => {
    const swatch = swatchById('#1A2B3C');
    expect(swatch.id).toBe('#1A2B3C');
    expect(swatch.fg).toBe('#1A2B3C');
    expect(swatch.bg).toBe('rgba(26, 43, 60, 0.12)');
    expect(swatch.border).toBe('rgba(26, 43, 60, 0.4)');
  });

  it('accepts lowercase hex', () => {
    const swatch = swatchById('#ffffff');
    expect(swatch.fg).toBe('#ffffff');
    expect(swatch.bg).toBe('rgba(255, 255, 255, 0.12)');
    expect(swatch.border).toBe('rgba(255, 255, 255, 0.4)');
  });

  it('falls back to neutral for a malformed hex', () => {
    expect(swatchById('#FFF').id).toBe('neutral');
    expect(swatchById('#GGGGGG').id).toBe('neutral');
    expect(swatchById('#12345').id).toBe('neutral');
    expect(swatchById('#1234567').id).toBe('neutral');
  });
});
