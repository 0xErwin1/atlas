import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { custos, platform } from '@/api';
import type { components as CustosComponents } from '@/api/generated/custos.d.ts';
import type { components as PlatformComponents } from '@/api/generated/platform.d.ts';
import { errorHint } from '@/lib/apiError';
import { availableNavigationProviderIds } from '@/shell/navigationManifest';
import { useAuthStore } from '@/stores/auth';

export type DiscoverResponseDto = CustosComponents['schemas']['DiscoverResponseDto'];
export type ComponentSummaryDto = PlatformComponents['schemas']['ComponentSummaryDto'];

const INITIAL_RETRY_BACKOFF_MS = 5_000;
const MAX_RETRY_BACKOFF_MS = 60_000;

/**
 * Reusable source for "what can this shell compose" (E11-S8 design D6, PR4):
 * `GET /api/v2/platform/meta`'s present components and `GET
 * /api/v2/custos/discover`'s per-component scopes/admin flag. Fetched once
 * per session and reused by `AppRail.vue`, `SettingsView.vue`, and the
 * workspace store's `discoverableWorkspaces` — never re-fetched on every
 * mount, and refreshed only when the session actually changes (login after a
 * prior logout bumps `auth.sessionGeneration`).
 */
export const useDiscoveryStore = defineStore('discovery', () => {
  const discover = ref<DiscoverResponseDto | null>(null);
  const metaComponents = ref<ComponentSummaryDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  const auth = useAuthStore();
  let loadedSessionGeneration = -1;
  let loadGeneration = 0;
  let inFlight: Promise<void> | null = null;
  let retryBackoffMs = 0;
  let retryNotBefore = 0;

  const admin = computed(() => discover.value?.admin ?? false);
  const truncated = computed(() => discover.value?.truncated ?? false);

  /** Components `discover` found a grant or membership for — never a widening source. */
  const presentComponents = computed<Set<string>>(
    () => new Set((discover.value?.components ?? []).map((component) => component.component)),
  );

  /**
   * Provider ids the shell may compose right now (empty until the first fetch
   * settles, or forever empty when either source has nothing present — this
   * must never throw, only ever narrow to nothing).
   */
  const availableProviderIds = computed<Set<string>>(() =>
    availableNavigationProviderIds(metaComponents.value, presentComponents.value),
  );

  /** Records a failed attempt and doubles the backoff window (capped), so a broken endpoint never turns every navigation into a refetch. */
  function recordFailure(): void {
    retryBackoffMs =
      retryBackoffMs === 0 ? INITIAL_RETRY_BACKOFF_MS : Math.min(retryBackoffMs * 2, MAX_RETRY_BACKOFF_MS);
    retryNotBefore = Date.now() + retryBackoffMs;
  }

  function clearBackoff(): void {
    retryBackoffMs = 0;
    retryNotBefore = 0;
  }

  async function load(): Promise<void> {
    const requestGeneration = ++loadGeneration;
    const requestSessionGeneration = auth.sessionGeneration;
    loading.value = true;
    error.value = null;

    const results = await Promise.all([
      platform.GET('/api/v2/platform/meta', {}),
      custos.GET('/api/v2/custos/discover', {}),
    ]).catch((cause: unknown) => {
      if (requestGeneration === loadGeneration && requestSessionGeneration === auth.sessionGeneration) {
        error.value = errorHint(cause, 'Failed to load discoverable navigation');
      }
      return null;
    });

    if (requestGeneration !== loadGeneration || requestSessionGeneration !== auth.sessionGeneration) return;

    loading.value = false;

    if (results === null) {
      recordFailure();
      return;
    }

    const [metaResult, discoverResult] = results;

    if (metaResult.error !== undefined || metaResult.data === undefined) {
      error.value = errorHint(metaResult.error, 'Failed to load platform components');
      recordFailure();
      return;
    }

    if (discoverResult.error !== undefined || discoverResult.data === undefined) {
      error.value = errorHint(discoverResult.error, 'Failed to load discoverable navigation');
      recordFailure();
      return;
    }

    metaComponents.value = metaResult.data.components;
    discover.value = discoverResult.data;
    loadedSessionGeneration = requestSessionGeneration;
    clearBackoff();
  }

  function startLoad(): Promise<void> {
    inFlight = load().finally(() => {
      inFlight = null;
    });
    return inFlight;
  }

  /**
   * Fetches once per session on success; a failure is never latched, only
   * throttled by a doubling backoff (capped). Never rejects — `load` catches
   * its own transport failures — so it is always safe to `await` from the
   * router guard. Concurrent callers share one in-flight request.
   */
  function ensureLoaded(): Promise<void> {
    if (loadedSessionGeneration === auth.sessionGeneration) return Promise.resolve();
    if (inFlight !== null) return inFlight;
    if (Date.now() < retryNotBefore) return Promise.resolve();

    return startLoad();
  }

  /** Clears the backoff and refetches immediately, regardless of the retry window. */
  function retry(): Promise<void> {
    clearBackoff();
    return inFlight ?? startLoad();
  }

  /** Drops the cached response so the next `ensureLoaded` refetches (called on logout). */
  function reset(): void {
    discover.value = null;
    metaComponents.value = [];
    error.value = null;
    loadedSessionGeneration = -1;
    loadGeneration += 1;
    inFlight = null;
    clearBackoff();
  }

  return {
    discover,
    metaComponents,
    loading,
    error,
    admin,
    truncated,
    presentComponents,
    availableProviderIds,
    load,
    ensureLoaded,
    retry,
    reset,
  };
});
