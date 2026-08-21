/**
 * A small string-keyed memo with a fixed capacity. Reads refresh an entry's
 * recency and inserts past the cap evict the least recently used entry, so a
 * long-lived cache of rendered output (diagrams, formulas) cannot grow without
 * bound over a session.
 */
export class BoundedCache<V> {
  private readonly entries = new Map<string, V>();

  constructor(private readonly cap: number) {}

  get size(): number {
    return this.entries.size;
  }

  get(key: string): V | undefined {
    const value = this.entries.get(key);
    if (value === undefined) return undefined;

    this.entries.delete(key);
    this.entries.set(key, value);
    return value;
  }

  set(key: string, value: V): void {
    this.entries.delete(key);
    this.entries.set(key, value);

    if (this.entries.size <= this.cap) return;

    const oldest = this.entries.keys().next();
    if (!oldest.done) this.entries.delete(oldest.value);
  }

  clear(): void {
    this.entries.clear();
  }
}
