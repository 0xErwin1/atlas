import type { components } from '@/api/types.d.ts';
import { createBrowserPlatformTransport } from './browser';

export type MeResponse = components['schemas']['MeResponse'];

export interface DesktopConfiguration {
  origin: string;
}

export interface PlatformResult<T> {
  data?: T;
  error?: unknown;
}

export interface WorkspaceEventSource {
  readyState: number;
  onopen: ((event: Event) => void) | null;
  onerror: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  addEventListener: (type: string, listener: (event: Event) => void) => void;
  close: () => void;
}

export interface PlatformTransport {
  readonly isDesktop: boolean;
  login: (credentials: { username: string; password: string }) => Promise<PlatformResult<unknown>>;
  me: () => Promise<PlatformResult<MeResponse>>;
  resume: () => Promise<PlatformResult<MeResponse>>;
  logout: () => Promise<PlatformResult<unknown>>;
  getOrigin: () => Promise<PlatformResult<DesktopConfiguration>>;
  setOrigin: (origin: string) => Promise<PlatformResult<DesktopConfiguration>>;
  createWorkspaceEventSource: (workspaceSlug: string) => WorkspaceEventSource;
}

let platformTransport = createBrowserPlatformTransport();

export function setPlatformTransport(transport: PlatformTransport): void {
  platformTransport = transport;
}

export function getPlatformTransport(): PlatformTransport {
  return platformTransport;
}

export function resetPlatformTransportForTest(): void {
  platformTransport = createBrowserPlatformTransport();
}
