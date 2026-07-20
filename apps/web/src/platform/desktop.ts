import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { decodeDesktopHttpResponse } from './fetch';
import type {
  DesktopConfiguration,
  DesktopPreferences,
  MeResponse,
  PlatformResult,
  PlatformTransport,
  WorkspaceEventSource,
} from './transport';

interface DesktopEvent<T> {
  payload: T;
}

export interface DesktopBridge {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  listen: <T>(event: string, handler: (event: DesktopEvent<T>) => void) => Promise<() => void>;
}

interface NormalizedDesktopEvent {
  event_type: string;
  data: unknown;
}

interface DesktopWorkspaceClosed {
  workspace_slug: string;
}

const DESKTOP_EVENT_NAME = 'atlas://workspace-event';
const DESKTOP_CLOSED_EVENT_NAME = 'atlas://workspace-closed';
const DESKTOP_RESYNC_EVENT_NAME = 'atlas://workspace-resync';
const DESKTOP_SESSION_ACTION_EVENT_NAME = 'atlas://session-action';
const DESKTOP_GATE_OBSERVATION_KEY = '__atlasDesktopGateLiveUpdateObservation';
const DESKTOP_GATE_OBSERVER_KEY = Symbol.for('atlas.desktop.gate.live-updates');
const CONNECTING = 0;
const OPEN = 1;
const CLOSED = 2;

export type DesktopGateLiveUpdateStatus =
  | 'event'
  | 'reconnect-failed'
  | 'reconnected'
  | 'reconnecting'
  | 'resync';

export interface DesktopGateLiveUpdateObservation {
  count: number;
  eventType?: string;
  status: DesktopGateLiveUpdateStatus;
  workspaceSlug?: string;
}

export interface DesktopGateLiveUpdateObserver {
  recordEvent: (eventType: string, workspaceSlug: string) => void;
  recordStatus: (status: Exclude<DesktopGateLiveUpdateStatus, 'event'>) => void;
  snapshot: () => DesktopGateLiveUpdateObservation;
  subscribe: (listener: (observation: DesktopGateLiveUpdateObservation) => void) => () => void;
}

interface DesktopGateObservationWindow extends Window {
  [DESKTOP_GATE_OBSERVATION_KEY]?: Pick<DesktopGateLiveUpdateObserver, 'snapshot' | 'subscribe'>;
}

type DesktopGateObserverRegistry = { [key: symbol]: DesktopGateLiveUpdateObserver | undefined };

const desktopGateObserverRegistry = globalThis as DesktopGateObserverRegistry;

export function createDesktopGateLiveUpdateObserver(enabled: boolean): DesktopGateLiveUpdateObserver {
  let observation: DesktopGateLiveUpdateObservation = { count: 0, status: 'resync' };
  const listeners = new Set<(next: DesktopGateLiveUpdateObservation) => void>();

  function publish(next: DesktopGateLiveUpdateObservation): void {
    observation = next;
    listeners.forEach((listener) => {
      listener(observation);
    });
  }

  const observer: DesktopGateLiveUpdateObserver = {
    recordEvent(eventType, workspaceSlug): void {
      publish({
        count: observation.count + 1,
        eventType,
        status: 'event',
        workspaceSlug,
      });
    },
    recordStatus(status): void {
      publish({ count: observation.count, status });
    },
    snapshot(): DesktopGateLiveUpdateObservation {
      return observation;
    },
    subscribe(listener): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };

  if (enabled && typeof window !== 'undefined') {
    const gateWindow = window as DesktopGateObservationWindow;
    gateWindow[DESKTOP_GATE_OBSERVATION_KEY] = {
      snapshot: observer.snapshot,
      subscribe: observer.subscribe,
    };
  }

  return observer;
}

const desktopGateLiveUpdateObserver =
  import.meta.env.VITE_ATLAS_DESKTOP_GATE === '1' ? createDesktopGateLiveUpdateObserver(true) : null;

if (desktopGateLiveUpdateObserver !== null) {
  desktopGateObserverRegistry[DESKTOP_GATE_OBSERVER_KEY] = desktopGateLiveUpdateObserver;
}

export function getDesktopGateLiveUpdateObserver(): DesktopGateLiveUpdateObserver | null {
  return desktopGateLiveUpdateObserver;
}

class DesktopWorkspaceEventSource implements WorkspaceEventSource {
  readonly atlasDesktopEventSource = true;
  readyState = CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  private readonly listeners = new Map<string, Set<(event: Event) => void>>();
  private unlisten: (() => void) | null = null;
  private unlistenClosed: (() => void) | null = null;
  private unlistenResync: (() => void) | null = null;
  private readonly bridge: DesktopBridge;
  private readonly workspaceSlug: string;

  constructor(bridge: DesktopBridge, workspaceSlug: string) {
    this.bridge = bridge;
    this.workspaceSlug = workspaceSlug;
    void bridge
      .listen<NormalizedDesktopEvent>(DESKTOP_EVENT_NAME, (event) => this.dispatch(event.payload))
      .then((unlisten) => {
        if (this.readyState === CLOSED) {
          unlisten();
          return;
        }

        this.unlisten = unlisten;
        this.readyState = OPEN;
        this.onopen?.(new Event('open'));
      })
      .catch(() => this.fail());

    void bridge
      .listen<DesktopWorkspaceClosed>(DESKTOP_CLOSED_EVENT_NAME, (event) => {
        if (event.payload.workspace_slug === workspaceSlug) this.fail();
      })
      .then((unlisten) => {
        if (this.readyState === CLOSED) {
          unlisten();
          return;
        }

        this.unlistenClosed = unlisten;
      })
      .catch(() => this.fail());

    void bridge
      .listen<DesktopWorkspaceClosed>(DESKTOP_RESYNC_EVENT_NAME, (event) => {
        if (event.payload.workspace_slug === workspaceSlug) this.dispatchResync();
      })
      .then((unlisten) => {
        if (this.readyState === CLOSED) {
          unlisten();
          return;
        }

        this.unlistenResync = unlisten;
      })
      .catch(() => this.fail());

    void bridge.invoke<void>('desktop_workspace_events_subscribe', { workspaceSlug }).catch(() => {
      this.fail();
    });
  }

  addEventListener(type: string, listener: (event: Event) => void): void {
    const listeners = this.listeners.get(type) ?? new Set();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  close(): void {
    if (this.readyState === CLOSED) return;
    this.readyState = CLOSED;
    this.detachAndStop();
  }

  private detachAndStop(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.unlistenClosed?.();
    this.unlistenClosed = null;
    this.unlistenResync?.();
    this.unlistenResync = null;
    void this.bridge.invoke<void>('desktop_workspace_events_stop', { workspaceSlug: this.workspaceSlug });
  }

  private dispatch(payload: NormalizedDesktopEvent): void {
    if (this.readyState === CLOSED) return;

    const event = new MessageEvent(payload.event_type, { data: JSON.stringify(payload) });
    if (payload.event_type === 'message') {
      this.onmessage?.(event);
      return;
    }

    this.listeners.get(payload.event_type)?.forEach((listener) => {
      listener(event);
    });
  }

  private fail(): void {
    if (this.readyState === CLOSED) return;
    this.readyState = CLOSED;
    this.detachAndStop();
    this.onerror?.(new Event('error'));
  }

  private dispatchResync(): void {
    if (this.readyState === CLOSED) return;

    const event = new Event('resync');
    this.listeners.get('resync')?.forEach((listener) => {
      listener(event);
    });
  }
}

function desktopBridge(): DesktopBridge {
  return { invoke, listen };
}

async function readDesktopClipboardImage(bridge: DesktopBridge): Promise<File | null> {
  try {
    const framed = await bridge.invoke<ArrayBuffer>('desktop_read_clipboard_image');
    const { meta, body } = decodeDesktopHttpResponse(framed);
    if (meta.status !== 200) return null;

    const mime =
      meta.headers.find(([name]) => name.toLowerCase() === 'content-type')?.[1] ?? 'image/png';
    const extension = mime === 'image/png' ? 'png' : (mime.split('/')[1] ?? 'bin');

    return new File([body], `pasted-image.${extension}`, { type: mime });
  } catch {
    return null;
  }
}

export function createDesktopPlatformTransport(bridge: DesktopBridge = desktopBridge()): PlatformTransport {
  void bridge
    .listen(DESKTOP_SESSION_ACTION_EVENT_NAME, (event) => {
      window.dispatchEvent(new CustomEvent('atlas:session-action', { detail: event.payload }));
    })
    .catch((cause) => {
      console.error('desktop: session action listener registration failed', cause);
    });

  return {
    isDesktop: true,
    login(credentials) {
      return bridge.invoke<PlatformResult<unknown>>('desktop_auth_login', { credentials });
    },
    me() {
      return bridge.invoke<PlatformResult<MeResponse>>('desktop_auth_me');
    },
    resume() {
      return bridge.invoke<PlatformResult<MeResponse>>('desktop_auth_resume');
    },
    logout() {
      return bridge.invoke<PlatformResult<unknown>>('desktop_auth_logout');
    },
    getOrigin() {
      return bridge.invoke<PlatformResult<DesktopConfiguration>>('desktop_get_origin');
    },
    setOrigin(origin) {
      return bridge.invoke<PlatformResult<DesktopConfiguration>>('desktop_set_origin', { origin });
    },
    getWindowDecorations() {
      return bridge.invoke<PlatformResult<DesktopPreferences>>('desktop_get_window_decorations');
    },
    setWindowDecorations(decorations) {
      return bridge.invoke<PlatformResult<DesktopPreferences>>('desktop_set_window_decorations', {
        decorations,
      });
    },
    getZoom() {
      return bridge.invoke<PlatformResult<DesktopPreferences>>('desktop_get_zoom');
    },
    setZoom(zoomFactor) {
      return bridge.invoke<PlatformResult<DesktopPreferences>>('desktop_set_zoom', { zoomFactor });
    },
    createWorkspaceEventSource(workspaceSlug) {
      return new DesktopWorkspaceEventSource(bridge, workspaceSlug);
    },
    readClipboardImage() {
      return readDesktopClipboardImage(bridge);
    },
  };
}
