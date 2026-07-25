import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getPlatformTransport } = vi.hoisted(() => ({ getPlatformTransport: vi.fn() }));

vi.mock('@/platform/transport', () => ({ getPlatformTransport }));

import { saveDownload } from '@/lib/download';

function blob(): Blob {
  return { arrayBuffer: () => Promise.resolve(new Uint8Array([1, 2, 3]).buffer) } as Blob;
}

describe('saveDownload', () => {
  beforeEach(() => {
    getPlatformTransport.mockReset();
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();
  });

  it('saves through an anchor in the browser', async () => {
    getPlatformTransport.mockReturnValue({ isDesktop: false });
    const click = vi.fn();
    const anchor = { href: '', download: '', click } as unknown as HTMLAnchorElement;
    vi.spyOn(document, 'createElement').mockReturnValueOnce(anchor);

    await expect(saveDownload(blob(), 'report.pdf')).resolves.toBe(true);

    expect(anchor.download).toBe('report.pdf');
    expect(anchor.href).toBe('blob:1');
    expect(click).toHaveBeenCalled();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:1');
  });

  it('hands the bytes to the host on desktop, where an anchor cannot save a file', async () => {
    const saveDownloadCommand = vi.fn().mockResolvedValue({ data: { path: '/home/u/report.pdf' } });
    getPlatformTransport.mockReturnValue({ isDesktop: true, saveDownload: saveDownloadCommand });

    await expect(saveDownload(blob(), 'report.pdf')).resolves.toBe(true);

    expect(saveDownloadCommand).toHaveBeenCalledWith('report.pdf', new Uint8Array([1, 2, 3]));
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });

  it('reports a failed host save instead of pretending the file landed', async () => {
    getPlatformTransport.mockReturnValue({
      isDesktop: true,
      saveDownload: vi.fn().mockResolvedValue({ error: 'the download could not be saved' }),
    });

    await expect(saveDownload(blob(), 'report.pdf')).resolves.toBe(false);
  });
});
