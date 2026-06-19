import { describe, expect, it } from 'vitest';
import { activeDotIndex, dotScrollTarget } from '@/lib/kanbanDots';

describe('activeDotIndex', () => {
  it('returns 0 when there is a single column or none', () => {
    expect(activeDotIndex(0, 0, 1)).toBe(0);
    expect(activeDotIndex(120, 500, 1)).toBe(0);
    expect(activeDotIndex(0, 0, 0)).toBe(0);
  });

  it('returns 0 when the track does not overflow', () => {
    expect(activeDotIndex(0, 0, 3)).toBe(0);
  });

  it('maps the scroll fraction to the nearest column index', () => {
    // 3 columns -> indices 0,1,2 across maxScroll 600
    expect(activeDotIndex(0, 600, 3)).toBe(0);
    expect(activeDotIndex(300, 600, 3)).toBe(1);
    expect(activeDotIndex(600, 600, 3)).toBe(2);
    expect(activeDotIndex(450, 600, 3)).toBe(2);
    expect(activeDotIndex(140, 600, 3)).toBe(0);
  });

  it('clamps out-of-range scroll positions', () => {
    expect(activeDotIndex(-50, 600, 3)).toBe(0);
    expect(activeDotIndex(9999, 600, 3)).toBe(2);
  });
});

describe('dotScrollTarget', () => {
  it('returns 0 for the first dot or a single column', () => {
    expect(dotScrollTarget(0, 600, 3)).toBe(0);
    expect(dotScrollTarget(0, 600, 1)).toBe(0);
  });

  it('spreads dots evenly across the scrollable range', () => {
    expect(dotScrollTarget(1, 600, 3)).toBe(300);
    expect(dotScrollTarget(2, 600, 3)).toBe(600);
  });

  it('clamps the index to the valid range', () => {
    expect(dotScrollTarget(5, 600, 3)).toBe(600);
    expect(dotScrollTarget(-2, 600, 3)).toBe(0);
  });
});
