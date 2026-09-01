import { FAQ_PAGES } from "@/config/faq";
import { site } from "@/config/site";

type NavItem = { href: string; label: string };
type NavSection = { label: string; items: NavItem[] };

export const navSections: NavSection[] = [
  {
    label: "Library",
    items: [
      { href: "/", label: "Home" },
      { href: "/download", label: "Download" },
      { href: "/liberator", label: "Liberator" },
    ],
  },
  {
    label: "System",
    items: [
      { href: "/settings", label: "Settings" },
      { href: "/updates", label: "Updates" },
    ],
  },
  {
    label: "Help",
    items: [{ href: "/faq", label: "FAQ" }],
  },
];

type Crumb = { label: string; href: string };

const PATH_LABELS: Record<string, string> = Object.fromEntries(
  Object.entries(FAQ_PAGES).map(([slug, page]) => [
    slug === "index" ? "/faq" : `/faq/${slug}`,
    page.title,
  ]),
);

export function normalizePath(path: string): string {
  if (!path) return "/";
  const trimmed = path.replace(/\/+$/, "");
  return trimmed === "" ? "/" : trimmed;
}

export function resolveCrumbHref(
  href: string,
  hasLocal: boolean | null,
): string {
  const target = normalizePath(href);
  return target === "/" && hasLocal === false ? "/download" : target;
}

export function isActivePath(href: string, pathname: string): boolean {
  const target = normalizePath(href);
  const current = normalizePath(pathname);
  if (target === "/") return current === "/";
  return current === target || current.startsWith(`${target}/`);
}

function segmentLabel(segment: string): string {
  return segment
    .split("-")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

export function breadcrumbFor(path: string): Crumb[] {
  const normalized = normalizePath(path);
  const crumbs: Crumb[] = [{ label: site.name, href: "/" }];
  if (normalized === "/") {
    crumbs.push({ label: "Home", href: "/" });
    return crumbs;
  }
  let href = "";
  for (const segment of normalized.split("/").filter(Boolean)) {
    href += `/${segment}`;
    crumbs.push({ label: PATH_LABELS[href] ?? segmentLabel(segment), href });
  }
  return crumbs;
}
