import {
  DEFAULT_ZOOM_FACTOR,
  MAX_ZOOM_FACTOR,
  MIN_ZOOM_FACTOR,
  type PlatformTransport,
  ZOOM_FACTOR_STEP,
} from './transport';

type ZoomTransport = Pick<PlatformTransport, 'getZoom' | 'setZoom'>;

const ZOOM_HOTKEYS = ['=', '+', '-', '0'];

function clampZoom(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_ZOOM_FACTOR;
  return Math.min(MAX_ZOOM_FACTOR, Math.max(MIN_ZOOM_FACTOR, value));
}

function isZoomHotkey(event: KeyboardEvent): boolean {
  return (event.ctrlKey || event.metaKey) && ZOOM_HOTKEYS.includes(event.key);
}

function nextZoomForKey(key: string, current: number): number {
  switch (key) {
    case '=':
    case '+':
      return clampZoom(current + ZOOM_FACTOR_STEP);
    case '-':
      return clampZoom(current - ZOOM_FACTOR_STEP);
    default:
      return DEFAULT_ZOOM_FACTOR;
  }
}

/**
 * Installs the desktop webview zoom hotkeys (Ctrl/Cmd with +/-/0) and keeps the persisted
 * zoom factor in sync through the platform transport. The stored value is the source of
 * truth: `current` only ever advances to what the host reported back, and every hotkey is
 * serialized behind the initial zoom fetch so a press during startup steps from the
 * persisted factor rather than the placeholder default. Returns a teardown function that
 * removes the listener, mirroring `installTransportStatus`.
 */
export function installDesktopZoom(transport: ZoomTransport): () => void {
  let current = DEFAULT_ZOOM_FACTOR;
  let pending: Promise<void> = transport
    .getZoom()
    .then((result) => {
      if (result.data !== undefined) current = result.data.zoom_factor;
    })
    .catch(() => {
      current = DEFAULT_ZOOM_FACTOR;
    });

  function applyHotkey(key: string): Promise<void> {
    const next = nextZoomForKey(key, current);
    if (next === current) return Promise.resolve();

    return transport
      .setZoom(next)
      .then((result) => {
        if (result.data !== undefined) current = result.data.zoom_factor;
      })
      .catch(() => {
        // Best-effort hotkey: on a host failure keep the last synced zoom factor.
      });
  }

  const onKeydown = (event: KeyboardEvent): void => {
    if (!isZoomHotkey(event)) return;

    event.preventDefault();
    const key = event.key;
    pending = pending.then(() => applyHotkey(key));
  };

  window.addEventListener('keydown', onKeydown);
  return () => {
    window.removeEventListener('keydown', onKeydown);
  };
}
