"use client";

import { useEffect, useMemo, useState } from "react";
import { usePathname, useRouter } from "next/navigation";
import { DiskSpaceModal } from "./DiskSpaceModal";
import { Sidebar } from "./Sidebar";
import { SteamLoginModal } from "./SteamLoginModal";
import { Toasts } from "./Toasts";
import { Topbar } from "./Topbar";
import { ScrollReveal } from "./ScrollReveal";
import { applyAccent, DEFAULT_FILL, DEFAULT_STRIPE } from "@/config/accents";
import { breadcrumbFor, normalizePath } from "@/config/nav";
import { onBridgeEvent, useInfo, useSettings } from "@/lib/bridge";
import { DetailContext, type DetailCrumb } from "@/lib/detail";
import { resetPlatformView } from "@/lib/platform-view";
import { RATE_LIMIT_TOAST, showToast } from "@/lib/toast";
import { hasOpenModal } from "@/components/Modal";
import { hasOpenInfoHint } from "@/components/InfoHint";

const ERROR_TARGETS: [string, string | undefined][] = [
  ["downloader", undefined],
  ["rvpn", undefined],
  ["launch", undefined],
  ["liberator", "liberator"],
  ["update", undefined],
];

function BridgeToasts() {
  const settings = useSettings();
  const info = useInfo();

  useEffect(() => {
    if (info?.warning) showToast(info.warning);
  }, [info?.warning]);

  useEffect(() => {
    if (!settings) return;
    const onInvalid = (_field: string, message: string) => showToast(message);
    const onLoggedOut = (_kind: string, message: string) => showToast(message);
    const onCacheCleared = () => showToast("Cache cleared");
    settings.settings_error.connect(onInvalid);
    settings.logged_out.connect(onLoggedOut);
    settings.cache_cleared.connect(onCacheCleared);
    return () => {
      settings.settings_error.disconnect(onInvalid);
      settings.logged_out.disconnect(onLoggedOut);
      settings.cache_cleared.disconnect(onCacheCleared);
    };
  }, [settings]);

  useEffect(
    () =>
      onBridgeEvent("downloader", (event, args) => {
        if (event === "done") {
          const outcome = args[1] as string;
          const code = (args[0] as string).split("_")[0];
          if (outcome === "done") showToast(`${code} installed`);
          else if (outcome === "verified") showToast(`${code} verified`);
          else if (outcome === "switched_hm")
            showToast(`${code} switched to Heated Metal`);
          else if (outcome === "switched_tb")
            showToast(`${code} switched to Throwback`);
          else if (outcome === "no_space")
            showToast(`${code} download failed, ran out of disk space`);
        } else if (event === "partial_deleted") {
          if (!args[2]) showToast(args[3] as string);
        } else if (event === "rate_limited") {
          showToast(args[0] as string, { key: RATE_LIMIT_TOAST });
        } else if (event === "warning") {
          showToast(args[0] as string);
        }
      }),
    [],
  );

  useEffect(() => {
    const offs = ERROR_TARGETS.map(([target, key]) =>
      onBridgeEvent(target, (event, args) => {
        if (event === "error") showToast(args[0] as string, { key });
      }),
    );
    return () => offs.forEach((off) => off());
  }, []);

  useEffect(
    () =>
      onBridgeEvent("update", (event, args) => {
        if (event === "done") {
          const name = args[1] as string;
          const message = args[2] as string;
          if (args[0]) showToast(`${name} updated`);
          else showToast(message || `${name} update failed`);
        }
      }),
    [],
  );

  return null;
}

const bar =
  "block h-0.5 w-4.5 rounded-sm bg-text transition-transform duration-200 ease-in-out";

export function AppShell({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const [detail, setDetail] = useState<DetailCrumb | null>(null);
  const detailStore = useMemo(() => ({ detail, setDetail }), [detail]);
  const pathname = usePathname();
  const router = useRouter();
  const settings = useSettings();

  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--bar-fill", settings?.bar_fill || DEFAULT_FILL);
    root.style.setProperty(
      "--bar-stripe",
      settings?.bar_stripe || DEFAULT_STRIPE,
    );
    applyAccent(settings?.accent ?? "");
  }, [settings?.bar_fill, settings?.bar_stripe, settings?.accent]);

  useEffect(() => {
    document.documentElement.toggleAttribute(
      "data-reduce-motion",
      !!settings?.reduce_motion,
    );
  }, [settings?.reduce_motion]);

  useEffect(() => {
    if (!normalizePath(pathname).startsWith("/faq")) resetPlatformView();
  }, [pathname]);

  useEffect(() => {
    function clean() {
      const hash = window.location.hash.slice(1);
      if (!hash) return;
      requestAnimationFrame(() => {
        if (!document.getElementById(hash))
          window.history.replaceState(
            window.history.state,
            "",
            window.location.pathname + window.location.search,
          );
      });
    }
    clean();
    window.addEventListener("hashchange", clean);
    return () => window.removeEventListener("hashchange", clean);
  }, [pathname]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement
      )
        return;
      if (hasOpenModal() || hasOpenInfoHint()) return;
      if (open) {
        setOpen(false);
        return;
      }
      const intercepted = new CustomEvent("throwback:back", {
        cancelable: true,
      });
      window.dispatchEvent(intercepted);
      if (intercepted.defaultPrevented) return;
      const current = normalizePath(pathname);
      const crumbs = breadcrumbFor(current);
      const parent = crumbs.length > 1 ? crumbs[crumbs.length - 2].href : null;
      if (!parent) return;
      const destination = normalizePath(parent);
      if (destination === current) return;
      router.push(destination);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pathname, router, open]);

  useEffect(() => {
    if (!open) return;
    const breakpoint = getComputedStyle(document.documentElement)
      .getPropertyValue("--breakpoint-nav")
      .trim();
    const media = window.matchMedia(`(max-width: ${breakpoint})`);
    const onChange = () => {
      if (!media.matches) setOpen(false);
    };
    onChange();
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onClick(event: MouseEvent) {
      const sidebar = document.getElementById("sidebar");
      const hamburger = document.getElementById("hamburger");
      const target = event.target as Node;
      if (
        sidebar &&
        !sidebar.contains(target) &&
        hamburger &&
        !hamburger.contains(target)
      ) {
        setOpen(false);
      }
    }
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, [open]);

  return (
    <DetailContext value={detailStore}>
      <button
        id="hamburger"
        type="button"
        aria-label="Toggle navigation menu"
        aria-expanded={open}
        aria-controls="sidebar"
        onClick={() => setOpen((value) => !value)}
        className="fixed left-3 top-3 z-(--z-hamburger) hidden flex-col gap-1 px-2 py-2.5 max-nav:flex"
      >
        <span
          className={`${bar} ${open ? "translate-y-[3px] rotate-45" : ""}`}
        />
        <span
          className={`${bar} ${open ? "-translate-y-[3px] -rotate-45" : ""}`}
        />
      </button>

      <div className="flex min-h-screen">
        <Sidebar open={open} onNavigate={() => setOpen(false)} />
        <div className="ml-(--sidebar-w) flex min-h-screen min-w-0 flex-1 flex-col overflow-y-clip max-nav:ml-0">
          <Topbar />
          <main
            tabIndex={-1}
            key={pathname}
            className="w-full flex-1 animate-fade-up p-(--page-pad) outline-none"
          >
            {children}
          </main>
        </div>
      </div>

      <Toasts />
      <BridgeToasts />
      <SteamLoginModal />
      <DiskSpaceModal />
      <ScrollReveal />
    </DetailContext>
  );
}
