import {
  DEFAULT_ZOOM_FACTOR,
  MAX_ZOOM_FACTOR,
  MIN_ZOOM_FACTOR,
  type PlatformTransport,
  ZOOM_FACTOR_STEP,
} from './transport';

type ZoomTransport = Pick<PlatformTransport, 'getZoom' | 'setZoom'>;

function clampZoom(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_ZOOM_FACTOR;
  return Math.min(MAX_ZOOM_FACTOR, Math.max(MIN_ZOOM_FACTOR, value));
}

function nextZoomForEvent(event: KeyboardEvent, current: number): number | null {
  if (!(event.ctrlKey || event.metaKey)) return null;

  switch (event.key) {
    case '=':
    case '+':
      return clampZoom(current + ZOOM_FACTOR_STEP);
    case '-':
      return clampZoom(current - ZOOM_FACTOR_STEP);
    case '0':
      return DEFAULT_ZOOM_FACTOR;
    default:
      return null;
  }
}

/**
 * Installs the desktop webview zoom hotkeys (Ctrl/Cmd with +/-/0) and keeps the persisted
 * zoom factor in sync through the platform transport. The stored value is the source of
 * truth: `current` only ever advances to what the host reported back. Returns a teardown
 * function that removes the listener, mirroring `installTransportStatus`.
 */
export function installDesktopZoom(transport: ZoomTransport): () => void {
  let current = DEFAULT_ZOOM_FACTOR;

  void transport
    .getZoom()
    .then((result) => {
      if (result.data !== undefined) current = result.data.zoom_factor;
    })
    .catch(() => {
      current = DEFAULT_ZOOM_FACTOR;
    });

  const onKeydown = (event: KeyboardEvent): void => {
    const next = nextZoomForEvent(event, current);
    if (next === null) return;

    event.preventDefault();
    if (next === current) return;

    void transport
      .setZoom(next)
      .then((result) => {
        if (result.data !== undefined) current = result.data.zoom_factor;
      })
      .catch(() => {
        // Best-effort hotkey: on a host failure keep the last synced zoom factor.
      });
  };

  window.addEventListener('keydown', onKeydown);
  return () => {
    window.removeEventListener('keydown', onKeydown);
  };
}
