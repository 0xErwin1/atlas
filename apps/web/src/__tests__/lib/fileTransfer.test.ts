import { describe, expect, it } from 'vitest';
import {
  attachmentFileName,
  filesFromClipboard,
  filesFromDataTransfer,
  isImageFile,
} from '@/lib/fileTransfer';

const png = (name: string) => new File([new Uint8Array([1])], name, { type: 'image/png' });

describe('fileTransfer', () => {
  it('filesFromDataTransfer returns the dropped files in order', () => {
    const dt = { files: [png('a.png'), png('b.png')] } as unknown as DataTransfer;

    expect(filesFromDataTransfer(dt).map((f) => f.name)).toEqual(['a.png', 'b.png']);
  });

  it('filesFromDataTransfer tolerates a null transfer', () => {
    expect(filesFromDataTransfer(null)).toEqual([]);
  });

  it('filesFromClipboard keeps only file items, skipping strings', () => {
    const image = png('shot.png');
    const data = {
      items: [
        { kind: 'string', getAsFile: () => null },
        { kind: 'file', getAsFile: () => image },
      ],
    } as unknown as DataTransfer;

    expect(filesFromClipboard(data)).toEqual([image]);
  });

  it('isImageFile distinguishes images from other types', () => {
    expect(isImageFile(png('x.png'))).toBe(true);
    expect(isImageFile(new File([''], 'notes.txt', { type: 'text/plain' }))).toBe(false);
  });

  it('attachmentFileName keeps a normal ascii name', () => {
    expect(attachmentFileName(png('diagram.png'))).toBe('diagram.png');
  });

  it('attachmentFileName synthesizes a name for a nameless clipboard image', () => {
    expect(attachmentFileName(png(''))).toBe('pasted-image.png');
  });

  it('attachmentFileName folds non-ascii characters and drops quotes for header safety', () => {
    const name = attachmentFileName(png('café"1.png'));

    expect(name).toBe('caf_1.png');
    expect(name).not.toContain('"');
  });
});
