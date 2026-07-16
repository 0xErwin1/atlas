import createClient, { type Middleware } from 'openapi-fetch';
import { fetchThroughPlatform } from '@/platform/fetch';
import type { paths } from './types.d.ts';

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS', 'TRACE']);
const LOCALLY_HANDLED_UNAUTHORIZED_PATHS = new Set(['/api/auth/login', '/api/auth/me']);
const CACHEABLE_RESOURCE_TYPES = new Set(['documents', 'folders', 'boards', 'tasks', 'projects']);

let unauthorizedHandler: (() => void | Promise<void>) | undefined;
export type RequestOutcome = 'start' | 'success' | 'failure';
let requestOutcomeHandler: ((outcome: RequestOutcome) => void) | undefined;
export interface CacheInvalidationScope {
  status: 403 | 404;
  scope: 'workspace' | 'resource' | 'none';
  workspaceSlug: string | null;
  tags: string[];
}

let cacheInvalidationHandler: ((scope: CacheInvalidationScope) => void | Promise<void>) | undefined;

export const csrfMiddlewareForTest: Middleware = {
  onRequest({ request }) {
    if (!SAFE_METHODS.has(request.method)) {
      const modified = request.clone();
      modified.headers.set('X-Atlas-CSRF', '1');
      return modified;
    }
  },
};

export const unauthorizedMiddlewareForTest: Middleware = {
  onResponse({ request, response }) {
    if (response.status === 401 && !LOCALLY_HANDLED_UNAUTHORIZED_PATHS.has(new URL(request.url).pathname)) {
      return unauthorizedHandler?.();
    }
  },
};

export const cacheInvalidationMiddlewareForTest: Middleware = {
  onResponse({ request, response }) {
    if (response.status === 403 || response.status === 404) {
      return cacheInvalidationHandler?.(
        scopeCacheInvalidation(response.status, new URL(request.url).pathname),
      );
    }
  },
};

export const requestOutcomeMiddlewareForTest: Middleware = {
  onRequest() {
    requestOutcomeHandler?.('start');
  },
  onResponse({ response }) {
    requestOutcomeHandler?.(response.status >= 500 ? 'failure' : 'success');
  },
  onError() {
    requestOutcomeHandler?.('failure');
  },
};

export function setUnauthorizedHandler(handler: () => void | Promise<void>): void {
  unauthorizedHandler = handler;
}

export function setCacheInvalidationHandler(
  handler: (scope: CacheInvalidationScope) => void | Promise<void>,
): void {
  cacheInvalidationHandler = handler;
}

export function setRequestOutcomeHandler(handler: ((outcome: RequestOutcome) => void) | undefined): void {
  requestOutcomeHandler = handler;
}

function scopeCacheInvalidation(status: 403 | 404, path: string): CacheInvalidationScope {
  const parts = path.split('/').filter(Boolean);
  const workspaceIndex = parts.indexOf('workspaces');
  const workspaceSlug = workspaceIndex === -1 ? null : (parts[workspaceIndex + 1] ?? null);
  const candidateResourceType = workspaceIndex === -1 ? null : parts[workspaceIndex + 2];
  const hasResourcePath = candidateResourceType !== undefined && candidateResourceType !== null;
  const resourceType =
    typeof candidateResourceType === 'string' && CACHEABLE_RESOURCE_TYPES.has(candidateResourceType)
      ? candidateResourceType
      : null;
  const resourceId = resourceType === null ? null : (parts[workspaceIndex + 3] ?? null);

  return {
    status,
    scope:
      workspaceSlug === null
        ? 'none'
        : resourceType && resourceId
          ? 'resource'
          : resourceType || !hasResourcePath
            ? 'workspace'
            : 'none',
    workspaceSlug,
    tags: resourceType && resourceId ? [`${singularResourceType(resourceType)}:${resourceId}`] : [],
  };
}

function singularResourceType(resourceType: string): string {
  return resourceType.endsWith('s') ? resourceType.slice(0, -1) : resourceType;
}

export const wrappedClient = createClient<paths>({
  baseUrl: globalThis.location?.origin ?? '',
  credentials: 'include',
  fetch: fetchThroughPlatform,
});

wrappedClient.use(csrfMiddlewareForTest);
wrappedClient.use(unauthorizedMiddlewareForTest);
wrappedClient.use(cacheInvalidationMiddlewareForTest);
wrappedClient.use(requestOutcomeMiddlewareForTest);
