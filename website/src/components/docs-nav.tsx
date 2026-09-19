"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { DocSection } from "@/lib/docs";

function href(slug: string) {
  return slug === "quick-start" ? "/docs/" : `/docs/${slug}/`;
}

function List({ sections, current }: { sections: DocSection[]; current: string }) {
  return (
    <>
      {sections.map((section) => (
        <div key={section.title} className="mb-6">
          <p className="mb-2 font-mono text-[12.5px] text-faint">{section.title}</p>
          <ul className="grid gap-0.5 border-l border-line">
            {section.items.map((item) => {
              const active = href(item.slug) === current;
              return (
                <li key={item.slug}>
                  <Link
                    href={href(item.slug)}
                    aria-current={active ? "page" : undefined}
                    className={`-ml-px block border-l py-1 pl-3.5 ${
                      active ? "border-accent text-ink" : "border-transparent text-muted"
                    }`}
                  >
                    {item.title}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </>
  );
}

// A sidebar on wide screens; a closed <details> on narrow ones, which works without JavaScript.
export function DocsNav({ sections }: { sections: DocSection[] }) {
  const path = usePathname();
  const current = path.endsWith("/") ? path : `${path}/`;
  return (
    <nav aria-label="Documentation" className="text-sm">
      <div className="sticky top-6 max-md:hidden">
        <List sections={sections} current={current} />
      </div>
      <details className="rounded-md border border-line bg-surface md:hidden">
        <summary className="cursor-pointer px-3.5 py-2.5 font-medium">All pages</summary>
        <div className="px-3.5 pt-1 pb-0.5">
          <List sections={sections} current={current} />
        </div>
      </details>
    </nav>
  );
}
