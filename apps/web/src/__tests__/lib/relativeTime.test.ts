import { describe, expect, it } from 'vitest';
import { relativeTime } from '@/lib/relativeTime';

const NOW = new Date('2026-06-18T12:00:00Z').getTime();
const ago = (ms: number) => new Date(NOW - ms).toISOString();

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

describe('relativeTime', () => {
  it('shows "just now" under 45 seconds', () => {
    expect(relativeTime(ago(10 * SECOND), NOW)).toBe('just now');
  });

  it('shows minutes under an hour', () => {
    expect(relativeTime(ago(12 * MINUTE), NOW)).toBe('12m ago');
  });

  it('shows hours under a day', () => {
    expect(relativeTime(ago(2 * HOUR), NOW)).toBe('2h ago');
  });

  it('shows days under a week', () => {
    expect(relativeTime(ago(3 * DAY), NOW)).toBe('3d ago');
  });

  it('falls back to a locale date beyond a week', () => {
    const eightDaysAgo = ago(8 * DAY);
    expect(relativeTime(eightDaysAgo, NOW)).toBe(new Date(eightDaysAgo).toLocaleDateString());
  });

  it('returns the raw value for an unparseable input', () => {
    expect(relativeTime('not-a-date', NOW)).toBe('not-a-date');
  });
});
