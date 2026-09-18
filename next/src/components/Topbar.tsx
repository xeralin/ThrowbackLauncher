"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { BlinkCursor } from "@/components/ui";
import { breadcrumbFor, normalizePath } from "@/config/nav";
import { useDetail } from "@/lib/detail";

const linkClass =
  "cursor-pointer no-underline transition-colors hover:text-text";

export function Topbar() {
  const pathname = normalizePath(usePathname());
  const base = breadcrumbFor(pathname);
  const { detail } = useDetail();
  const lastIndex = base.length - 1;

  return (
    <div className="sticky top-0 z-(--z-topbar) flex h-(--topbar-h) items-center border-b border-border bg-surface px-(--page-pad) max-nav:pl-14 max-nav:pr-4">
      <div className="min-w-0 grow truncate-fade font-mono text-label tracking-[0.04em] text-text-muted">
        {base.map((crumb, index) => {
          if (index === lastIndex) {
            return detail ? (
              <span key={index}>
                <button
                  type="button"
                  onClick={detail.reset}
                  className={linkClass}
                >
                  {crumb.label}
                </button>
                {" / "}
              </span>
            ) : (
              <span key={index} className="text-text">
                {crumb.label}
              </span>
            );
          }
          return (
            <span key={index}>
              <Link
                href={crumb.href}
                onClick={(event) => {
                  if (detail && crumb.href === pathname) {
                    event.preventDefault();
                    detail.reset();
                  }
                }}
                className={linkClass}
              >
                {crumb.label}
              </Link>
              {" / "}
            </span>
          );
        })}
        {detail && <span className="text-text">{detail.label}</span>}
        <BlinkCursor />
      </div>
      <div id="topbar-actions" className="flex min-w-0 items-center" />
    </div>
  );
}
