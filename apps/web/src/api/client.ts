import createClient from 'openapi-fetch';
import type { paths } from './types.d.ts';

export const apiClient = createClient<paths>({
  baseUrl: '',
  credentials: 'include',
  fetch: (req) => globalThis.fetch(req),
});
