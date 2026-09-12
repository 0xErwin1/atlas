import { describe, expect, it } from 'vitest';
import { DECLARED_NAVIGATION_PROVIDERS } from '@/shell/declaredNavigationProviders';
import {
  auditNavigationManifest,
  availableNavigationProviderIds,
  composeManifestEntries,
  NAVIGATION_MANIFEST,
} from '@/shell/navigationManifest';

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

describe('availableNavigationProviderIds (PR4 composer intersection)', () => {
  it('requires both sides: a provider is available only when its component is present in both meta and discover', () => {
    const metaComponents = [
      { stable_id: 'acta', navigation_providers: ['acta.workspace'] },
      { stable_id: 'custos', navigation_providers: ['custos.admin'] },
    ];

    const available = availableNavigationProviderIds(metaComponents, new Set(['acta']));

    expect(available).toEqual(new Set(['acta.workspace']));
  });

  it('treats a missing navigation_providers field as empty, never crashing', () => {
    const metaComponents = [{ stable_id: 'platform' }];

    const available = availableNavigationProviderIds(metaComponents, new Set(['platform']));

    expect(available).toEqual(new Set());
  });

  it('is empty when discover reports no present components, hiding everything without crashing', () => {
    const metaComponents = [
      { stable_id: 'acta', navigation_providers: ['acta.workspace'] },
      { stable_id: 'custos', navigation_providers: ['custos.admin'] },
    ];

    const available = availableNavigationProviderIds(metaComponents, new Set());

    expect(available).toEqual(new Set());
  });

  it('is empty when meta lists no components at all, even if discover has entries', () => {
    const available = availableNavigationProviderIds([], new Set(['acta', 'custos']));

    expect(available).toEqual(new Set());
  });
});

describe('composeManifestEntries (PR4 composer)', () => {
  it('returns only the rail entries whose id is available', () => {
    const entries = composeManifestEntries(NAVIGATION_MANIFEST, new Set(['acta.workspace']), 'rail');

    expect(entries).toEqual([{ id: 'acta.workspace', ...NAVIGATION_MANIFEST['acta.workspace'] }]);
  });

  it('returns only the settings entries whose id is available', () => {
    const entries = composeManifestEntries(NAVIGATION_MANIFEST, new Set(['custos.admin']), 'settings');

    expect(entries).toEqual([{ id: 'custos.admin', ...NAVIGATION_MANIFEST['custos.admin'] }]);
  });

  it('composes to an empty list when nothing is available, never throwing', () => {
    expect(composeManifestEntries(NAVIGATION_MANIFEST, new Set(), 'rail')).toEqual([]);
    expect(composeManifestEntries(NAVIGATION_MANIFEST, new Set(), 'settings')).toEqual([]);
  });
});
