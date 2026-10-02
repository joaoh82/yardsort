import type { MouseEvent, ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

/** The only addresses a link here goes to: the web, or mail. Anything else is just its words. */
const outward = (href: string | undefined): href is string =>
  !!href && /^(https?:\/\/|mailto:)/i.test(href);

/**
 * A link that opens in the user's browser. The window itself never goes anywhere: this text
 * was written by whoever opened a pull request or commented on one, which is to say by anyone.
 */
function Outward({ href, children }: { href: string; children: ReactNode }) {
  const open = (event: MouseEvent) => {
    event.preventDefault();
    void openUrl(href).catch(console.error);
  };
  return (
    <a
      href={href}
      title={href}
      onClick={open}
      // A middle click would otherwise ask the webview for a new window.
      onAuxClick={(event) => event.preventDefault()}
      className="text-accent underline decoration-accent/40 hover:decoration-accent"
    >
      {children}
    </a>
  );
}

const components: Components = {
  a: ({ href, children }) =>
    outward(href) ? <Outward href={href}>{children}</Outward> : <span>{children}</span>,
  // An image is a request to a server of the author's choosing, made the moment you read their
  // words; the app's content policy forbids it besides. So it is a link, with its description.
  img: ({ src, alt }) => {
    const label = alt?.trim() || "image";
    return outward(src) ? (
      <Outward href={src}>
        [image: {label}] <span aria-hidden>↗</span>
      </Outward>
    ) : (
      <span className="text-ink-faint">[image: {label}]</span>
    );
  },
  h1: ({ children }) => <h4 className="mt-4 mb-2 text-[15px] font-semibold">{children}</h4>,
  h2: ({ children }) => <h5 className="mt-4 mb-2 text-[14px] font-semibold">{children}</h5>,
  h3: ({ children }) => <h6 className="mt-3 mb-1.5 font-semibold">{children}</h6>,
  h4: ({ children }) => <h6 className="mt-3 mb-1.5 font-semibold">{children}</h6>,
  h5: ({ children }) => <h6 className="mt-3 mb-1.5 font-medium">{children}</h6>,
  h6: ({ children }) => <h6 className="mt-3 mb-1.5 font-medium">{children}</h6>,
  p: ({ children }) => <p className="my-2 leading-relaxed">{children}</p>,
  ul: ({ children, className }) => (
    <ul
      className={`my-2 pl-5 ${className?.includes("contains-task-list") ? "list-none" : "list-disc"}`}
    >
      {children}
    </ul>
  ),
  ol: ({ children }) => <ol className="my-2 list-decimal pl-5">{children}</ol>,
  li: ({ children, className }) => (
    <li className={`my-0.5 ${className?.includes("task-list-item") ? "-ml-5" : ""}`}>{children}</li>
  ),
  // A task list's boxes say what was ticked; they are not there to be ticked from here.
  input: ({ checked }) => (
    <input
      type="checkbox"
      checked={!!checked}
      disabled
      readOnly
      className="mr-1.5 align-middle accent-(--color-accent)"
    />
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-line pl-3 text-ink-muted">{children}</blockquote>
  ),
  code: ({ children }) => (
    <code className="rounded bg-raised px-1 py-0.5 font-mono text-[12px]">{children}</code>
  ),
  pre: ({ children }) => (
    <pre className="my-2 overflow-x-auto rounded bg-raised p-3 font-mono text-[12px] leading-relaxed [&>code]:bg-transparent [&>code]:p-0">
      {children}
    </pre>
  ),
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="border-collapse text-left">{children}</table>
    </div>
  ),
  th: ({ children }) => <th className="border border-line px-2 py-1 font-medium">{children}</th>,
  td: ({ children }) => <td className="border border-line px-2 py-1">{children}</td>,
  hr: () => <hr className="my-3 border-line" />,
};

/**
 * Markdown somebody else wrote, shown safely: GitHub's flavour (tables, task lists,
 * strikethrough, bare links), no raw HTML at all, no image loaded, and every link opened in the
 * browser rather than here.
 */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="min-w-0 break-words select-text [&>*:first-child]:mt-0 [&>*:last-child]:mb-0">
      <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}
