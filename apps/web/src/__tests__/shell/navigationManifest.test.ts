import { describe, expect, it } from 'vitest';
import { DECLARED_NAVIGATION_PROVIDERS } from '@/shell/declaredNavigationProviders';
import { auditNavigationManifest, NAVIGATION_MANIFEST } from '@/shell/navigationManifest';

describe('NAVIGATION_MANIFEST bidirectional audit (INV-MANIFEST-BIDIRECTIONAL)', () => {
  it('has exactly one manifest entry per declared provider id, with no gap', () => {
    const result = auditNavigationManifest(NAVIGATION_MANIFEST, DECLARED_NAVIGATION_PROVIDERS);

    expect(result.fabricatedKeys).toEqual([]);
    expect(result.undeclaredIds).toEqual([]);
    expect(result.ok).toBe(true);
  });

  it('fails when the manifest carries an id no component declares (fabricated-id probe)', () => {
    const withFabricated = {
      ...NAVIGATION_MANIFEST,
      'fabricated.provider': {
        label: 'Fabricated',
        icon: 'question',
        surface: 'rail' as const,
        routeName: 'notes',
      },
    };

    const result = auditNavigationManifest(withFabricated, DECLARED_NAVIGATION_PROVIDERS);

    expect(result.fabricatedKeys).toEqual(['fabricated.provider']);
    expect(result.ok).toBe(false);
  });

  it('fails when a declared id has no manifest entry', () => {
    const { 'custos.admin': _removed, ...withoutCustosAdmin } = NAVIGATION_MANIFEST;

    const result = auditNavigationManifest(withoutCustosAdmin, DECLARED_NAVIGATION_PROVIDERS);

    expect(result.undeclaredIds).toEqual(['custos.admin']);
    expect(result.ok).toBe(false);
  });

  it('lifts the Acta rail entry verbatim from the pre-existing AppRail item', () => {
    const acta = NAVIGATION_MANIFEST['acta.workspace'];

    expect(acta).toEqual({
      label: 'Acta',
      icon: 'files',
      surface: 'rail',
      routeName: 'notes',
      activeRoutes: ['notes', 'tasks', 'task-view', 'task-detail', 'search', 'files'],
    });
  });

  it('resolves the custos admin provider to the settings surface', () => {
    const custos = NAVIGATION_MANIFEST['custos.admin'];

    expect(custos).toBeDefined();
    expect(custos?.surface).toBe('settings');
    expect(custos?.routeName).toBe('settings');
  });
});
