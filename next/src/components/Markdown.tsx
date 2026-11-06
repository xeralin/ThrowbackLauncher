import type { RootContent } from "hast";
import type { ComponentProps } from "react";
import { toJsxRuntime, type Components } from "hast-util-to-jsx-runtime";
import { Fragment, jsx, jsxs } from "react/jsx-runtime";
import { ExternalLink } from "@/components/ExternalLink";
import { CHECK, CHEVRON_RIGHT, StrokeIcon } from "@/components/icons";
import { Note } from "@/components/Note";
import { link, stepBox } from "@/components/ui";
import { OPENABLE, parseInline, toAlert } from "@/lib/markdown";

const ALERTS: Record<string, ComponentProps<typeof Note>["variant"]> = {
  NOTE: "info",
  TIP: "tip",
  IMPORTANT: "important",
  WARNING: "notice",
  CAUTION: "error",
};

const mdLink = `${link} [&_code]:text-inherit [&_strong]:text-inherit`;
const components: Partial<Components> = {
  a: ({ href, id, title, children }) =>
    href && OPENABLE.test(href) ? (
      <ExternalLink href={href} id={id} title={title} className={mdLink}>
        {children}
      </ExternalLink>
    ) : href?.startsWith("#") ? (
      <a href={href} id={id} title={title} className={mdLink}>
        {children}
      </a>
    ) : (
      <span id={id} title={title}>
        {children}
      </span>
    ),
  img: ({ src, title, width, height }) => (
    <img
      src={src || undefined}
      alt=""
      title={title}
      width={width}
      height={height}
      loading="lazy"
    />
  ),
  input: ({ checked }) => (
    <span
      className={`-ml-4 mr-1.5 inline-flex size-3.5 items-center justify-center rounded-sm border align-[-2px] ${
        checked
          ? "border-action bg-action text-action-text"
          : "border-border bg-surface-2"
      }`}
    >
      {checked && <StrokeIcon d={CHECK} className="size-3 [&]:stroke-[3]" />}
    </span>
  ),
  summary: ({ children }) => (
    <summary>
      <StrokeIcon d={CHEVRON_RIGHT} className="size-3.5 shrink-0" />
      <span>{children}</span>
    </summary>
  ),
  table: ({ children }) => (
    <div className="w-fit max-w-full overflow-x-auto rounded-lg border border-border">
      <table>{children}</table>
    </div>
  ),
  blockquote: ({ node, children }) => {
    const alert = node && toAlert(node);
    if (!alert) return <blockquote>{children}</blockquote>;
    return (
      <Note variant={ALERTS[alert.kind]} className="[&>*+*]:mt-2">
        {renderNodes(alert.children)}
      </Note>
    );
  },
};

function renderNodes(nodes: RootContent[]) {
  return toJsxRuntime(
    { type: "root", children: nodes },
    { Fragment, jsx, jsxs, components, passNode: true },
  );
}

export function renderInline(text: string) {
  return renderNodes(parseInline(text));
}

export function Markdown({ nodes }: { nodes: RootContent[] }) {
  return <div className={`notes ${stepBox} bg-well`}>{renderNodes(nodes)}</div>;
}
