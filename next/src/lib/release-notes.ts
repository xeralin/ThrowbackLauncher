import type { Element, ElementContent, Root } from "hast";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkBreaks from "remark-breaks";
import remarkGemoji from "remark-gemoji";
import remarkGfm from "remark-gfm";
import remarkGithub, {
  defaultBuildUrl,
  type BuildUrlValues,
} from "remark-github";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";
import { visitParents } from "unist-util-visit-parents";
import {
  OPENABLE,
  isBlank,
  limitNotes,
  type ReleaseNotes,
} from "@/lib/markdown";

function buildUrl(values: BuildUrlValues) {
  return values.type === "commit" || values.type === "compare"
    ? false
    : defaultBuildUrl(values);
}

const schema = {
  ...defaultSchema,
  clobberPrefix: "",
  strip: [...(defaultSchema.strip ?? []), "style", "iframe"],
  tagNames: [
    ...(defaultSchema.tagNames ?? []).filter(
      (tag) => tag !== "picture" && tag !== "source",
    ),
    "mark",
  ],
};

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

function resolveUrls(repository: string) {
  const base = `https://github.com/${repository}/`;
  return (tree: Root) => {
    visitParents(tree, "element", (node: Element) => {
      if (node.tagName !== "a" && node.tagName !== "img") return;
      const key = node.tagName === "a" ? "href" : "src";
      const value = node.properties[key];
      if (
        typeof value !== "string" ||
        !value ||
        SCHEME.test(value) ||
        value.startsWith("#")
      )
        return;
      const kind = key === "src" ? "raw/HEAD/" : "blob/HEAD/";
      node.properties[key] = URL.parse(value, base + kind)?.href ?? value;
    });
  };
}

const GITHUB_IMAGE =
  /^https:\/\/([a-z0-9-]+\.)*(github\.com|githubusercontent\.com)\//i;

const BLOCKS = new Set(["details", "blockquote", "li", "td", "th", "section"]);

function soleChild(node: Element | Root, tagName: string): Element | null {
  const children = node.children.filter((child) => !isBlank(child));
  const [only] = children;
  return children.length === 1 &&
    only.type === "element" &&
    only.tagName === tagName
    ? only
    : null;
}

function linkImages(tree: Root) {
  visitParents(tree, "element", (node: Element, ancestors) => {
    if (node.tagName !== "img") return;
    const src = node.properties.src;
    if (typeof src !== "string" || !src) return;
    const parent = ancestors[ancestors.length - 1];
    const shown: ElementContent = GITHUB_IMAGE.test(src)
      ? node
      : { type: "text", value: String(node.properties.alt || src) };
    const link = ancestors.find(
      (ancestor): ancestor is Element =>
        ancestor.type === "element" && ancestor.tagName === "a",
    );
    if (link) {
      if (!OPENABLE.test(String(link.properties.href)))
        link.properties.href = src;
      parent.children.splice(parent.children.indexOf(node), 1, shown);
      return;
    }
    const wrapped: Element = {
      type: "element",
      tagName: "a",
      properties: { href: src },
      children: [shown],
    };
    const block =
      parent.type === "root" ||
      (parent.type === "element" && BLOCKS.has(parent.tagName));
    const siblings = parent.children.filter((child) => !isBlank(child));
    const at = siblings.indexOf(node);
    const alone =
      block &&
      siblings.every((child) => child.type !== "text") &&
      ![siblings[at - 1], siblings[at + 1]].some(
        (sibling) =>
          sibling?.type === "element" &&
          (sibling.tagName === "img" || sibling.tagName === "a"),
      );
    parent.children.splice(
      parent.children.indexOf(node),
      1,
      alone
        ? { type: "element", tagName: "p", properties: {}, children: [wrapped] }
        : wrapped,
    );
  });
  visitParents(tree, "element", (node: Element) => {
    const link = soleChild(node, "a");
    if (link && soleChild(link, "img")) node.properties.dataFigure = "";
  });
}

export function parseReleaseNotes(
  body: string,
  repository?: string,
): ReleaseNotes {
  const processor = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkBreaks)
    .use(remarkGemoji)
    .use(repository ? [[remarkGithub, { repository, buildUrl }]] : [])
    .use(remarkRehype, { allowDangerousHtml: true })
    .use(rehypeRaw)
    .use(rehypeSanitize, schema)
    .use(repository ? [[resolveUrls, repository]] : [])
    .use(() => linkImages);
  const text = body.slice(0, 16_000);
  const notes = limitNotes(processor.runSync(processor.parse(text)) as Root);
  return text.length < body.length ? { ...notes, truncated: true } : notes;
}
