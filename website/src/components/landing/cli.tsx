import Link from "next/link";
import { docUrl } from "@/lib/docs";

// The commands the landing page shows off: one line each, in the order you would meet them.
const LINES: { command: string; note: string }[] = [
  {
    command: 'ys workspace new yardsort "fix the flaky login test"',
    note: "branch, worktree and an agent, in one line",
  },
  {
    command: "ys attach fix-the-flaky-login-test",
    note: "put it back on your terminal; Ctrl-] detaches",
  },
  {
    command: "ys logs fix-the-flaky-login-test",
    note: "how it ended up, as plain text — finished ones too",
  },
  { command: "ys workspace list --json", note: "every command takes --json, for scripts" },
];

export function Cli() {
  return (
    <section aria-labelledby="h-cli" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <div className="grid items-start gap-12 md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <div>
          <p className="mb-2.5 font-mono text-[12.5px] text-accent">05 · Command line</p>
          <h3 id="h-cli" className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">
            Start an agent without opening the window
          </h3>
          <div className="mt-3.5 space-y-3 text-muted">
            <p>
              <code className="font-mono text-[13.5px] text-ink">ys</code> is a small command-line
              client, a separate download in each release. It reads the same database as the app, so
              each sees the other&apos;s work.
            </p>
            <p>
              The agent it starts belongs to the background process, not to the command — so it
              carries on after <code className="font-mono text-[13.5px] text-ink">ys</code> returns,
              and you can attach to it later, from a terminal or from the window.
            </p>
            <p>
              <Link href={`${docUrl("guide/cli")}/`} className="link text-ink">
                The <code className="font-mono text-[13.5px]">ys</code> command line
              </Link>{" "}
              has every command.
            </p>
          </div>
        </div>
        <div className="overflow-hidden rounded-[10px] border border-line bg-surface">
          <ul className="divide-y divide-line font-mono text-[13px]">
            {LINES.map((line) => (
              <li key={line.command} className="px-4 py-3.5">
                <code className="block break-words">
                  <span className="text-faint select-none">$ </span>
                  {line.command}
                </code>
                <span className="mt-1 block text-[12px] text-faint">{line.note}</span>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </section>
  );
}
