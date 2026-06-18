/**
 * Format an ISO timestamp as a short relative label ("12m ago", "2h ago",
 * "3d ago"). Falls back to a locale date beyond a week, and returns the raw
 * input unchanged when it isn't a parseable date.
 */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;

  const seconds = Math.max(0, Math.floor((now - then) / 1000));

  if (seconds < 45) return 'just now';

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;

  return new Date(then).toLocaleDateString();
}
