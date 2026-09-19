import type { ReactNode } from "react";
import { DocsNav } from "@/components/docs-nav";
import { DOCS_NAV } from "@/lib/docs";

export default function DocsLayout({ children }: { children: ReactNode }) {
  return (
    <div className="mx-auto grid max-w-[1120px] gap-x-12 gap-y-8 px-5 py-10 md:grid-cols-[13rem_minmax(0,1fr)] md:px-8 md:py-14">
      <DocsNav sections={DOCS_NAV} />
      {children}
    </div>
  );
}
