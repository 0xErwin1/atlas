import type { components } from '@/api/types.d.ts';
import { createBrowserPlatformTransport } from './browser';

export type MeResponse = components['schemas']['MeResponse'];

export interface DesktopConfiguration {
  origin: string;
}

export interface DesktopPreferences {
  window_decorations: boolean;
  zoom_factor: number;
}

export const DEFAULT_ZOOM_FACTOR = 1.0;
export const MIN_ZOOM_FACTOR = 0.5;
export const MAX_ZOOM_FACTOR = 3.0;
export const ZOOM_FACTOR_STEP = 0.1;

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
  getWindowDecorations: () => Promise<PlatformResult<DesktopPreferences>>;
  setWindowDecorations: (decorations: boolean) => Promise<PlatformResult<DesktopPreferences>>;
  getZoom: () => Promise<PlatformResult<DesktopPreferences>>;
  setZoom: (zoomFactor: number) => Promise<PlatformResult<DesktopPreferences>>;
  createWorkspaceEventSource: (workspaceSlug: string) => WorkspaceEventSource;
  /**
   * Reads an image off the native clipboard, or resolves to `null` when the
   * clipboard holds no image or the platform cannot read it. Only the desktop host
   * implements this; the browser relies on the `ClipboardEvent` file items instead.
   */
  readClipboardImage: () => Promise<File | null>;
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
