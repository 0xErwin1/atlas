/**
 * Splits a raw markdown string into a YAML frontmatter metadata object and body.
 *
 * Frontmatter is a YAML block delimited by `---` on the first line and a
 * closing `---` on a subsequent line. If no valid block is found the full
 * string is returned as body with an empty metadata map.
 *
 * The YAML parser handles the subset commonly found in Atlas document
 * frontmatter: flat key/value pairs plus single-level sequences written as
 * `- item` lines. Nested objects are not supported and are left as strings.
 */
export function splitFrontmatter(raw: string): { body: string; meta: Record<string, unknown> } {
  if (!raw.startsWith('---\n') && raw !== '---') {
    return { body: raw, meta: {} };
  }

  const closeIndex = raw.indexOf('\n---', 3);
  if (closeIndex === -1) {
    return { body: raw, meta: {} };
  }

  const yamlBlock = raw.slice(4, closeIndex);
  const afterClose = raw.slice(closeIndex + 4);
  const body = afterClose.startsWith('\n') ? afterClose.slice(1) : afterClose;
  const meta = parseSimpleYaml(yamlBlock);

  return { body, meta };
}

/**
 * Joins a metadata object and body back into a raw markdown string with a
 * YAML frontmatter block. When meta is empty the body is returned unchanged.
 */
export function joinFrontmatter(meta: Record<string, unknown>, body: string): string {
  if (Object.keys(meta).length === 0) {
    return body;
  }

  const yaml = serializeSimpleYaml(meta);
  return `---\n${yaml}---\n${body}`;
}

function parseSimpleYaml(yaml: string): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  const lines = yaml.split('\n');

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    if (line === undefined || line.trim() === '' || line.startsWith('#')) {
      i++;
      continue;
    }

    const colonPos = line.indexOf(':');
    if (colonPos === -1) {
      i++;
      continue;
    }

    const key = line.slice(0, colonPos).trim();
    const rawValue = line.slice(colonPos + 1).trim();

    if (rawValue === '') {
      const items: string[] = [];
      i++;

      while (i < lines.length && lines[i] !== undefined && /^\s+-/.test(lines[i] as string)) {
        const item = (lines[i] as string).replace(/^\s+-\s*/, '');
        items.push(item);
        i++;
      }

      result[key] = items;
      continue;
    }

    result[key] = parseScalar(rawValue);
    i++;
  }

  return result;
}

function parseScalar(value: string): unknown {
  if (value === 'true') return true;
  if (value === 'false') return false;
  if (value === 'null' || value === '~') return null;

  const num = Number(value);
  if (value !== '' && !Number.isNaN(num)) return num;

  if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
    return value.slice(1, -1);
  }

  return value;
}

function serializeSimpleYaml(meta: Record<string, unknown>): string {
  const lines: string[] = [];

  for (const [key, value] of Object.entries(meta)) {
    if (Array.isArray(value)) {
      lines.push(`${key}:`);
      for (const item of value) {
        lines.push(`  - ${String(item)}`);
      }
    } else if (value === null) {
      lines.push(`${key}: null`);
    } else if (typeof value === 'string') {
      lines.push(`${key}: ${value}`);
    } else {
      lines.push(`${key}: ${String(value)}`);
    }
  }

  return `${lines.join('\n')}\n`;
}
