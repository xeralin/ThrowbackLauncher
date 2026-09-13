import type { RootContent } from "hast";
import type { ComponentProps } from "react";
import { toJsxRuntime, type Components } from "hast-util-to-jsx-runtime";
import { Fragment, jsx, jsxs } from "react/jsx-runtime";
import { ExternalLink } from "@/components/ExternalLink";
import { StrokeIcon } from "@/components/icons";
import { Note } from "@/components/Note";
import { OPENABLE, parseInline, toAlert } from "@/lib/markdown";

const ALERTS: Record<string, [string, ComponentProps<typeof Note>["variant"]]> =
  {
    NOTE: ["Note", "info"],
    TIP: ["Tip", "tip"],
    IMPORTANT: ["Important", "important"],
    WARNING: ["Warning", "notice"],
    CAUTION: ["Caution", "error"],
  };

const link =
  "text-link hover:underline [&_code]:text-inherit [&_strong]:text-inherit";
const components: Partial<Components> = {
  a: ({ href, id, title, children }) =>
    href && OPENABLE.test(href) ? (
      <ExternalLink href={href} id={id} title={title} className={link}>
        {children}
      </ExternalLink>
    ) : href?.startsWith("#") ? (
      <a href={href} id={id} title={title} className={link}>
        {children}
      </a>
    ) : (
      <span id={id} title={title}>
        {children}
      </span>
    ),
  img: ({ src, alt, title, width, height }) => (
    <img
      src={src || undefined}
      alt={alt ?? ""}
      title={title}
      width={width}
      height={height}
      loading="lazy"
    />
  ),
  input: ({ checked }) => (
    <span
      role="checkbox"
      aria-checked={!!checked}
      aria-disabled
      className={`-ml-4 mr-1.5 inline-flex size-3.5 items-center justify-center rounded-[4px] border align-[-2px] ${
        checked
          ? "border-action bg-action text-action-text"
          : "border-border bg-surface-2"
      }`}
    >
      {checked && (
        <StrokeIcon d="M5 13l4 4L19 7" className="size-3 [&]:stroke-[3]" />
      )}
    </span>
  ),
  summary: ({ children }) => (
    <summary>
      <StrokeIcon d="m9 6 6 6-6 6" className="size-3.5 shrink-0" />
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
    const [title, variant] = ALERTS[alert.kind];
    return (
      <Note variant={variant}>
        <div className="mb-1 font-display text-[0.9rem] font-bold">{title}</div>
        <div className="[&>*+*]:mt-2">{renderNodes(alert.children)}</div>
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
  return <div className="notes">{renderNodes(nodes)}</div>;
}
