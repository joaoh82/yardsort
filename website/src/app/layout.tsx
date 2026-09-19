import type { Metadata } from "next";
import { Geist, JetBrains_Mono } from "next/font/google";
import type { ReactNode } from "react";
import { SiteFooter } from "@/components/site-footer";
import { SiteHeader } from "@/components/site-header";
import "./globals.css";

const geist = Geist({
  variable: "--font-geist",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
});
const jetbrains = JetBrains_Mono({
  variable: "--font-jetbrains",
  subsets: ["latin"],
  weight: ["400", "500"],
});

const description =
  "A desktop app for Linux, macOS and Windows that gives every task its own git worktree and its own terminal, running the coding agent of your choice.";

export const metadata: Metadata = {
  metadataBase: new URL("https://yardsort.sh"),
  title: { default: "Yardsort — run AI coding agents in parallel", template: "%s · Yardsort" },
  description,
  openGraph: {
    title: "Yardsort",
    description,
    images: ["/docs-images/overview.png"],
    type: "website",
  },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${geist.variable} ${jetbrains.variable} antialiased`}>
      <body className="font-sans">
        <SiteHeader />
        <main>{children}</main>
        <SiteFooter />
      </body>
    </html>
  );
}
