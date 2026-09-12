/**
 * Static shell navigation manifest, keyed by the `navigation_providers` id a
 * component declares in `reg5.rs` (E11-S8 design D-S8-4). Each entry is
 * inert metadata: this slice (PR2) neither reads nor composes it into
 * `AppRail.vue`/`SettingsView.vue` — that wiring is PR4. `acta.workspace`
 * is lifted verbatim from the pre-existing `AppRail.vue` rail item so PR4
 * is a pure composition change, not a re-derivation of labels/icons/routes.
 */

/** Where a resolved provider renders: the persistent rail, or a settings entry. */
export type NavigationSurface = 'rail' | 'settings';

export interface NavigationManifestEntry {
  label: string;
  icon: string;
  surface: NavigationSurface;
  routeName: string;
  /** Route names that also count as "current" for this entry, beyond `routeName`. */
  activeRoutes?: string[];
}

export const NAVIGATION_MANIFEST: Record<string, NavigationManifestEntry> = {
  'acta.workspace': {
    label: 'Acta',
    icon: 'files',
    surface: 'rail',
    routeName: 'notes',
    activeRoutes: ['notes', 'tasks', 'task-view', 'task-detail', 'search', 'files'],
  },
  'custos.admin': {
    label: 'Settings',
    icon: 'settings',
    surface: 'settings',
    routeName: 'settings',
  },
};

/** The subset of `ComponentSummaryDto` the PR4 composer needs (E11-S8 D6). */
export interface NavigationProviderComponent {
  stable_id: string;
  navigation_providers?: string[];
}

/**
 * Provider ids the shell may compose: declared by a present meta component
 * (its `navigation_providers`, an absent field treated as empty) AND whose
 * component `discover` also lists as present. Neither source decides alone —
 * a component present in the platform meta but absent from `discover` (no
 * grant, no membership) contributes no provider id, and vice versa.
 */
export function availableNavigationProviderIds(
  metaComponents: readonly NavigationProviderComponent[],
  presentComponents: ReadonlySet<string>,
): Set<string> {
  const available = new Set<string>();

  for (const component of metaComponents) {
    if (!presentComponents.has(component.stable_id)) continue;
    for (const providerId of component.navigation_providers ?? []) {
      available.add(providerId);
    }
  }

  return available;
}

export interface ComposedNavigationEntry extends NavigationManifestEntry {
  id: string;
}

/**
 * Filters `manifest` down to the entries for one surface whose id is in
 * `availableIds` (the PR4 composer's intersection). Returns each entry with
 * its manifest id attached, since the entry itself carries no id.
 */
export function composeManifestEntries(
  manifest: Record<string, NavigationManifestEntry>,
  availableIds: ReadonlySet<string>,
  surface: NavigationSurface,
): ComposedNavigationEntry[] {
  return Object.entries(manifest)
    .filter(([id, entry]) => entry.surface === surface && availableIds.has(id))
    .map(([id, entry]) => ({ id, ...entry }));
}

export interface NavigationManifestAuditResult {
  /** Manifest keys with no declaring component (INV-MANIFEST-BIDIRECTIONAL, "declared -> manifest" direction violated). */
  fabricatedKeys: string[];
  /** Declared ids with no manifest entry (the other direction). */
  undeclaredIds: string[];
  ok: boolean;
}

/**
 * Checks `manifest`'s keys against `declaredIds` in both directions
 * (INV-MANIFEST-BIDIRECTIONAL): every manifest key must have a declaring
 * component, and every declared id must have a manifest entry.
 */
export function auditNavigationManifest(
  manifest: Record<string, NavigationManifestEntry>,
  declaredIds: readonly string[],
): NavigationManifestAuditResult {
  const declared = new Set(declaredIds);
  const manifestKeys = new Set(Object.keys(manifest));

  const fabricatedKeys = [...manifestKeys].filter((key) => !declared.has(key)).sort();
  const undeclaredIds = [...declared].filter((id) => !manifestKeys.has(id)).sort();

  return {
    fabricatedKeys,
    undeclaredIds,
    ok: fabricatedKeys.length === 0 && undeclaredIds.length === 0,
  };
}
