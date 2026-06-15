import {
  schema as baseSchema,
  defaultMarkdownParser,
  defaultMarkdownSerializer,
  MarkdownParser,
  MarkdownSerializer,
} from 'prosemirror-markdown';
import type { Node as PmNode } from 'prosemirror-model';
import { Schema } from 'prosemirror-model';

/**
 * Extended ProseMirror schema that adds a `wikilink` inline node and a
 * `strikethrough` mark on top of the prosemirror-markdown base schema.
 *
 * The wikilink node stores the raw link title as an attribute and renders as
 * [[Title]] in serialized markdown.
 */
const atlasSchema = new Schema({
  nodes: (baseSchema.spec.nodes as unknown as { append: (spec: Record<string, unknown>) => unknown }).append({
    wikilink: {
      inline: true,
      attrs: { title: { default: '' } },
      group: 'inline',
      leaf: true,
      toDOM: (node: PmNode) => [
        'span',
        { class: 'wikilink', 'data-title': node.attrs.title },
        `[[${node.attrs.title as string}]]`,
      ],
      parseDOM: [
        {
          tag: 'span[data-title]',
          getAttrs: (dom: HTMLElement | string) => {
            if (typeof dom === 'string') return false;
            return { title: dom.getAttribute('data-title') ?? '' };
          },
        },
      ],
    },
  }),
  marks: (baseSchema.spec.marks as unknown as { append: (spec: Record<string, unknown>) => unknown }).append({
    strike: {
      toDOM: () => ['s', 0],
      parseDOM: [{ tag: 's' }, { tag: 'del' }, { tag: 'strike' }],
    },
  }),
});

/**
 * markdown-it inline rule that tokenizes [[Title]] as `wikilink` tokens.
 * The rule matches `[[` ... `]]` sequences and emits a single token per link.
 */
function wikilinkPlugin(md: {
  core: { ruler: { push: (name: string, fn: (state: unknown) => void) => void } };
}): void {
  md.core.ruler.push(
    'wikilink',
    (state: {
      tokens: Array<{
        type: string;
        children: Array<{
          type: string;
          attrSet?: (name: string, value: string) => void;
          content: string;
          level: number;
          nesting: number;
        }>;
      }>;
    }) => {
      for (const blockToken of state.tokens) {
        if (blockToken.type !== 'inline' || !blockToken.children) continue;

        const newChildren: typeof blockToken.children = [];

        for (const child of blockToken.children) {
          if (child.type !== 'text') {
            newChildren.push(child);
            continue;
          }

          const WIKILINK_RE = /\[\[([^\]]+)\]\]/g;
          let lastIndex = 0;
          const text = child.content;

          WIKILINK_RE.lastIndex = 0;

          for (const match of text.matchAll(WIKILINK_RE)) {
            if (match.index > lastIndex) {
              newChildren.push({
                type: 'text',
                content: text.slice(lastIndex, match.index),
                level: child.level,
                nesting: 0,
              });
            }

            const wlToken = {
              type: 'wikilink',
              content: match[1] ?? '',
              level: child.level,
              nesting: 0,
              attrSet: (_name: string, _value: string) => undefined,
            };

            newChildren.push(wlToken);
            lastIndex = (match.index ?? 0) + match[0].length;
          }

          if (lastIndex < text.length) {
            newChildren.push({
              type: 'text',
              content: text.slice(lastIndex),
              level: child.level,
              nesting: 0,
            });
          }
        }

        blockToken.children = newChildren;
      }
    },
  );
}

/**
 * Parser built from the atlas schema, extending the default markdown-it
 * tokenizer with the wikilink plugin and strikethrough, plus the default
 * parser token map with wikilink and strike rules.
 */
const atlasMarkdownParser = new MarkdownParser(
  atlasSchema,
  defaultMarkdownParser.tokenizer
    .use(wikilinkPlugin as unknown as Parameters<typeof defaultMarkdownParser.tokenizer.use>[0])
    .enable('strikethrough'),
  {
    ...defaultMarkdownParser.tokens,
    wikilink: {
      node: 'wikilink',
      getAttrs: (token) => ({ title: token.content }),
    },
    s: { mark: 'strike' },
  },
);

/**
 * Serializer built from the atlas schema, extending the default node/mark
 * serializers with a wikilink rule and a strike mark rule.
 */
const atlasMarkdownSerializer = new MarkdownSerializer(
  {
    ...defaultMarkdownSerializer.nodes,
    wikilink(state, node) {
      state.write(`[[${node.attrs.title as string}]]`);
    },
  },
  {
    ...defaultMarkdownSerializer.marks,
    strike: {
      open: '~~',
      close: '~~',
      mixable: true,
      expelEnclosingWhitespace: true,
    },
  },
);

/**
 * Parses a raw markdown string (without frontmatter) into a ProseMirror
 * document node using the atlas schema.
 */
export function markdownToDoc(md: string): PmNode {
  return atlasMarkdownParser.parse(md);
}

/**
 * Serializes a ProseMirror document node back to a markdown string.
 */
export function docToMarkdown(doc: PmNode): string {
  return atlasMarkdownSerializer.serialize(doc);
}
