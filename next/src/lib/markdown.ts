import type { Element, ElementContent, RootContent } from "hast";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";

export const OPENABLE = /^https?:\/\//i;

const plain = unified().use(remarkParse).use(remarkRehype).freeze();

export function parseInline(text: string): ElementContent[] {
  const root = plain.runSync(plain.parse(text));
  const first = root.children.find((node) => node.type === "element");
  return first?.tagName === "p" ? first.children : [];
}

export function isBlank(node: RootContent | ElementContent): boolean {
  return node.type === "text" && node.value.trim() === "";
}

const ALERT = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]$/i;

type Alert = { kind: string; children: ElementContent[] };

export function toAlert(blockquote: Element): Alert | null {
  const paragraph = blockquote.children.find((node) => !isBlank(node));
  if (paragraph?.type !== "element" || paragraph.tagName !== "p") return null;
  const [marker, ...rest] = paragraph.children;
  if (marker?.type !== "text") return null;
  const match = ALERT.exec(marker.value.trim());
  if (!match) return null;
  const body =
    rest[0]?.type === "element" && rest[0].tagName === "br"
      ? rest.slice(1)
      : rest;
  if (body[0]?.type === "text")
    body[0] = { ...body[0], value: body[0].value.replace(/^\n/, "") };
  const children = blockquote.children.filter(
    (node) => node !== paragraph && !isBlank(node),
  );
  if (body.length > 0) children.unshift({ ...paragraph, children: body });
  if (children.length === 0) return null;
  return { kind: match[1].toUpperCase(), children };
}
