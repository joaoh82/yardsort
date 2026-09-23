import Link from "next/link";
import { Detected, type Platform } from "@/components/platform";
import { docUrl } from "@/lib/docs";
import { appVersion, RELEASES_URL } from "@/lib/site";

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

// `ys` is not in the app bundle — it is its own archive per platform, named after the release, so
// the names are built from the version rather than written out and left to rot.
const DOWNLOADS: { platform: Platform; name: string; file: (version: string) => string }[] = [
  { platform: "linux", name: "Linux", file: (v) => `ys-${v}-linux-x86_64.tar.gz` },
  { platform: "mac", name: "macOS", file: (v) => `ys-${v}-macos-universal.tar.gz` },
  { platform: "win", name: "Windows", file: (v) => `ys-${v}-windows-x86_64.zip` },
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
          <div className="border-t border-line bg-raised px-4 py-3.5">
            <p className="text-[12.5px] text-muted">
              Not in the app bundle — its own download in{" "}
              <a href={RELEASES_URL} className="link text-ink">
                each release
              </a>
              . Unpack it and put it on your <code className="font-mono">PATH</code>.
            </p>
            <ul className="mt-2.5 space-y-1.5 font-mono text-[12.5px]">
              {DOWNLOADS.map((download) => (
                <li key={download.platform} className="flex flex-wrap items-baseline gap-x-2">
                  <span className="w-[62px] shrink-0 text-faint">{download.name}</span>
                  <code className="break-all">{download.file(appVersion())}</code>
                  <Detected platform={download.platform} />
                </li>
              ))}
            </ul>
            <p className="mt-2.5 text-[12px] text-faint">
              On macOS <code className="font-mono">ys</code> is unsigned, so a copy downloaded with
              a browser is quarantined:{" "}
              <code className="font-mono text-muted">xattr -d com.apple.quarantine ys</code>, or
              fetch it with <code className="font-mono text-muted">curl</code>.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
