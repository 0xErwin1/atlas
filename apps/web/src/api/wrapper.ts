import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './types.d.ts';

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS', 'TRACE']);
const LOCALLY_HANDLED_UNAUTHORIZED_PATHS = new Set(['/api/auth/login', '/api/auth/me']);

let unauthorizedHandler: (() => void) | undefined;

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
      unauthorizedHandler?.();
    }
  },
};

export function setUnauthorizedHandler(handler: () => void): void {
  unauthorizedHandler = handler;
}

export const wrappedClient = createClient<paths>({
  baseUrl: '',
  credentials: 'include',
  fetch: (req) => globalThis.fetch(req),
});

wrappedClient.use(csrfMiddlewareForTest);
wrappedClient.use(unauthorizedMiddlewareForTest);
