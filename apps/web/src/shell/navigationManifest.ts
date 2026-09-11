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
