import Link from "next/link";
import type { ReactNode } from "react";

export function CardGrid({ children }: { children: ReactNode }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(min(240px,100%),1fr))] gap-4 max-content:grid-cols-2 max-narrow:grid-cols-1">
      {children}
    </div>
  );
}

export function NavCard({
  href,
  title,
  description,
}: {
  href: string;
  title: ReactNode;
  description: ReactNode;
}) {
  return (
    <Link
      href={href}
      data-reveal
      className="group relative flex flex-col gap-2 overflow-hidden rounded-lg border border-border bg-surface p-5 text-text no-underline transition-[border-color,background-color,box-shadow] duration-200 hover:bg-surface-2 card-glow-hover card-line-hover"
    >
      <h3 className="font-display text-[1.05rem] font-bold text-text">
        {title}
      </h3>
      <div className="text-ui leading-normal text-text-muted">
        {description}
      </div>
    </Link>
  );
}
