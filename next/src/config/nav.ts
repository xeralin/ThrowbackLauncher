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

const PATH_LABELS: Record<string, string> = Object.fromEntries([
  ...navSections.flatMap((section) =>
    section.items.map((item) => [item.href, item.label]),
  ),
  ...Object.entries(FAQ_PAGES).map(([slug, page]) => [
    `/faq/${slug}`,
    page.title,
  ]),
]);

export function normalizePath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed === "" ? "/" : trimmed;
}

export function isActivePath(href: string, pathname: string): boolean {
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(`${href}/`);
}

export function breadcrumbFor(path: string): NavItem[] {
  const crumbs: NavItem[] = [{ label: site.name, href: "/" }];
  if (path === "/") {
    crumbs.push({ label: PATH_LABELS["/"], href: "/" });
    return crumbs;
  }
  let href = "";
  for (const segment of path.split("/").filter(Boolean)) {
    href += `/${segment}`;
    crumbs.push({ label: PATH_LABELS[href] ?? segment, href });
  }
  return crumbs;
}
