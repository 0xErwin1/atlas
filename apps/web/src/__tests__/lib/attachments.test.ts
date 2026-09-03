import { describe, expect, it } from 'vitest';
import { attachmentMarkdown, taskAttachmentContentUrl } from '@/lib/attachments';

describe('taskAttachmentContentUrl', () => {
  it('addresses the attachment content endpoint for the owning task', () => {
    expect(taskAttachmentContentUrl('acme', 'ATL-1', 'att-1')).toBe(
      '/api/v2/acta/workspaces/acme/tasks/ATL-1/attachments/att-1/content',
    );
  });

  it('escapes path segments so a slug with reserved characters cannot alter the path', () => {
    expect(taskAttachmentContentUrl('a/c me', 'ATL-1', 'att-1')).toBe(
      '/api/v2/acta/workspaces/a%2Fc%20me/tasks/ATL-1/attachments/att-1/content',
    );
  });
});

describe('attachmentMarkdown', () => {
  it('embeds an image by its extensionless name so it renders inline', () => {
    expect(attachmentMarkdown('diagram.png', 'image/png', '/c')).toBe('![diagram](/c)');
  });

  it('links a non-image by its full file name', () => {
    expect(attachmentMarkdown('report.pdf', 'application/pdf', '/c')).toBe('[report.pdf](/c)');
  });

  it('neutralizes characters that would break out of the link label', () => {
    expect(attachmentMarkdown('we]ird\nname.txt', 'text/plain', '/c')).toBe('[weird name.txt](/c)');
  });

  it('falls back to the file name when an image has no extension to strip', () => {
    expect(attachmentMarkdown('screenshot', 'image/png', '/c')).toBe('![screenshot](/c)');
  });
});
