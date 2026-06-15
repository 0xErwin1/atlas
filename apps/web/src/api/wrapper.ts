import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './types.d.ts';

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS', 'TRACE']);

export const csrfMiddlewareForTest: Middleware = {
  onRequest({ request }) {
    if (!SAFE_METHODS.has(request.method)) {
      const modified = request.clone();
      modified.headers.set('X-Atlas-CSRF', '1');
      return modified;
    }
  },
};

export const wrappedClient = createClient<paths>({
  baseUrl: '',
  credentials: 'include',
  fetch: (req) => globalThis.fetch(req),
});

wrappedClient.use(csrfMiddlewareForTest);
