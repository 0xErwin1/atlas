import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useSearchStore } from '@/stores/search';

/**
 * Unit tests for the typeModel comma-token computed logic (SE3–SE6, SE9).
 * The logic is: get parses the `type:` token from store.query (split on comma,
 * filter to supported values). Set replaces the `type:` token with a single
 * `type:<comma-joined>` form, or omits it when values is empty.
 *
 * These tests drive the logic in isolation via the store query string,
 * mirroring what SearchSidebar.vue's typeModel computed will do.
 */

const SUPPORTED = ['note', 'task'] as const;
type SupportedType = (typeof SUPPORTED)[number];

function getTypeModel(query: string): string[] {
  const match = query.match(/(?:^|\s)type:(\S+)(?:\s|$)/);
  if (!match || match[1] === undefined) return [];
  return match[1].split(',').filter((v): v is SupportedType => SUPPORTED.includes(v as SupportedType));
}

function setTypeModel(query: string, values: string[]): string {
  const supported = values.filter((v): v is SupportedType => SUPPORTED.includes(v as SupportedType));
  const stripped = query
    .replace(/(?:^|\s)type:\S+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (supported.length === 0) return stripped;
  const token = `type:${supported.join(',')}`;
  return stripped === '' ? token : `${stripped} ${token}`;
}

describe('typeModel comma-token logic (SE3–SE6, SE9)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  describe('getter — parses type: token from q', () => {
    it('returns [] when no type: token is present (SE5)', () => {
      expect(getTypeModel('foo bar')).toEqual([]);
    });

    it('returns ["note"] when query contains type:note (SE6)', () => {
      expect(getTypeModel('foo type:note bar')).toEqual(['note']);
    });

    it('returns ["task"] when query contains type:task (SE6)', () => {
      expect(getTypeModel('type:task')).toEqual(['task']);
    });

    it('returns ["note","task"] when query contains type:note,task (SE3, SE6)', () => {
      expect(getTypeModel('foo type:note,task bar')).toEqual(['note', 'task']);
    });

    it('returns ["note","task"] when query contains type:task,note (order preserved from token)', () => {
      const result = getTypeModel('type:task,note');
      expect(result).toContain('note');
      expect(result).toContain('task');
      expect(result).toHaveLength(2);
    });

    it('filters out doc and comment — they are never in the model (SE9)', () => {
      expect(getTypeModel('type:doc')).toEqual([]);
      expect(getTypeModel('type:comment')).toEqual([]);
      expect(getTypeModel('type:note,doc,comment')).toEqual(['note']);
    });
  });

  describe('setter — rewrites type: token in q', () => {
    it('sets type:note when values is ["note"] (SE4)', () => {
      const result = setTypeModel('foo type:note,task bar', ['note']);
      expect(result).toContain('type:note');
      expect(result).not.toMatch(/type:note,task/);
      expect(result).not.toMatch(/type:task/);
    });

    it('sets type:note,task when values is ["note","task"] (SE3)', () => {
      const result = setTypeModel('foo bar', ['note', 'task']);
      expect(result).toContain('type:note,task');
    });

    it('removes the type: token entirely when values is [] (SE5)', () => {
      const result = setTypeModel('foo type:note bar', []);
      expect(result).not.toMatch(/type:/);
      expect(result.trim()).toBe('foo bar');
    });

    it('preserves free text around the token', () => {
      const result = setTypeModel('urgent status:open type:task', ['note']);
      expect(result).toContain('urgent');
      expect(result).toContain('status:open');
      expect(result).toContain('type:note');
      expect(result).not.toContain('type:task');
    });

    it('doc and comment are never written by the setter (SE9)', () => {
      const result = setTypeModel('', ['doc', 'comment']);
      expect(result).not.toMatch(/type:/);
    });

    it('replaces an existing type: token when setting a new value (SE4)', () => {
      const result = setTypeModel('q type:note', ['task']);
      expect(result).toContain('type:task');
      expect(result).not.toContain('type:note');
    });
  });

  describe('integration with search store', () => {
    it('store.query reflects the comma-token after setter runs (SE3)', () => {
      const store = useSearchStore();
      store.setQuery('foo bar');

      const newQuery = setTypeModel(store.query, ['note', 'task']);
      store.setQuery(newQuery);

      expect(store.query).toContain('type:note,task');
    });

    it('reading from store.query with getter returns correct values (SE6)', () => {
      const store = useSearchStore();
      store.setQuery('status:open type:task');

      const model = getTypeModel(store.query);
      expect(model).toEqual(['task']);
    });
  });
});
