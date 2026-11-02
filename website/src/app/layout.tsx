import type { Metadata } from "next";
import localFont from "next/font/local";
import type { ReactNode } from "react";
import { CookieConsent } from "@/components/cookie-consent";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import { HOME_DESCRIPTION, HOME_TITLE, SITE_ORIGIN } from "@/lib/seo";
import "./globals.css";

const geist = localFont({
  src: "./fonts/geist-latin-wght-normal.woff2",
  variable: "--font-geist",
  weight: "400 600",
});
const jetbrains = localFont({
  src: "./fonts/jetbrains-mono-latin-wght-normal.woff2",
  variable: "--font-jetbrains",
  weight: "400 500",
});

export const metadata: Metadata = {
  metadataBase: new URL(SITE_ORIGIN),
  title: { default: HOME_TITLE, template: "%s · Yardsort" },
  description: HOME_DESCRIPTION,
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${jetbrains.variable} antialiased`}>
      <body className="font-sans">
        <SiteHeader />
        <main>{children}</main>
        <SiteFooter />
        <CookieConsent />
      </body>
    </html>
  );
}
