import type { Metadata, Viewport } from "next";
import Script from "next/script";
import { Rajdhani, Share_Tech_Mono, Barlow } from "next/font/google";
import { site } from "@/config/site";
import { PAGE_PAD_CLAMP, SIDEBAR_W_CLAMP, TOPBAR_H_PX } from "@/config/layout";
import { AppShell } from "@/components/AppShell";
import "./globals.css";

const display = Rajdhani({
  variable: "--font-rajdhani",
  subsets: ["latin"],
  weight: ["600", "700"],
  display: "swap",
  adjustFontFallback: false,
});

const mono = Share_Tech_Mono({
  variable: "--font-share-tech-mono",
  subsets: ["latin"],
  weight: ["400"],
  display: "swap",
  adjustFontFallback: false,
});

const body = Barlow({
  variable: "--font-barlow",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
  display: "swap",
  adjustFontFallback: false,
});

export const metadata: Metadata = {
  title: {
    default: site.name,
    template: `${site.name} | %s`,
  },
  description: site.description,
  applicationName: site.name,
};

export const viewport: Viewport = {
  themeColor: site.themeColor,
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html
      lang="en"
      className={`${display.variable} ${mono.variable} ${body.variable}`}
      style={
        {
          "--sidebar-w": SIDEBAR_W_CLAMP,
          "--topbar-h": `${TOPBAR_H_PX}px`,
          "--page-pad": PAGE_PAD_CLAMP,
        } as React.CSSProperties
      }
    >
      <body>
        <Script src="/bridge-init.js" strategy="afterInteractive" />
        <AppShell>{children}</AppShell>
      </body>
    </html>
  );
}
