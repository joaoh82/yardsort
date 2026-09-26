import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useState } from "react";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { openProjectFromDisk } from "@/features/sidebar/actions";
import { YsCommand, ysIsCurrent, ysSummary } from "@/features/ys/YsCommand";
import type { Preflight } from "@/lib/ipc";
import { usePreflightStore } from "@/stores/preflight";
import { useProjectsStore } from "@/stores/projects";

/** How to get git, per OS. Links and commands only; Yardsort never installs anything itself. */
const GIT_ADVICE: Record<string, { text: string; command?: string; url: string }> = {
  linux: {
    text: "Install it with your package manager — pacman, apt, dnf, zypper… For example:",
    command: "sudo pacman -S git",
    url: "https://git-scm.com/downloads/linux",
  },
  macos: {
    text: "Apple ships it with the command line tools:",
    command: "xcode-select --install",
    url: "https://git-scm.com/downloads/mac",
  },
  windows: {
    text: "Install Git for Windows, then start Yardsort again.",
    url: "https://git-scm.com/downloads/win",
  },
};

/**
 * The welcome screen. When something Yardsort needs is missing it says what, and how to get it;
 * when all is well on a first run it points at the one thing left to do. Otherwise it stays out
 * of the way.
 */
export function GettingStarted() {
  const report = usePreflightStore((s) => s.report);
  const hasProjects = useProjectsStore((s) => s.projects.length > 0);

  useEffect(() => void usePreflightStore.getState().check(), []);

  const needsHelp = report !== null && (!report.ready || report.env.warning !== null);
  return (
    // Centred when it fits; when it does not, it must scroll from the top. Centring the scroll
    // container itself would push the first lines out of reach above the fold.
    <div className="h-full overflow-y-auto">
      <div className="flex min-h-full items-center justify-center p-6">
        <div className="w-full max-w-xl">
          <div className="text-center">
            <img src="/icon.svg" alt="" className="mx-auto mb-4 size-16 opacity-90" />
            <h1 className="text-lg font-semibold">Yardsort</h1>
            <p className="mt-1 text-ink-muted">Every agent on its own track.</p>
          </div>
          {report && (needsHelp || !hasProjects) ? (
            <Checklist report={report} hasProjects={hasProjects} />
          ) : (
            <p className="mt-5 text-center text-ink-faint">
              {hasProjects
                ? "Pick a workspace on the left to start working."
                : "Add a project on the left to get started."}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function Checklist({ report, hasProjects }: { report: Preflight; hasProjects: boolean }) {
  const checking = usePreflightStore((s) => s.checking);
  const error = usePreflightStore((s) => s.error);
  const recheck = () => void usePreflightStore.getState().check(true);

  const agents = report.harnesses.filter((harness) => harness.enabled);
  const installed = agents.filter((harness) => harness.path);
  const gitAdvice = GIT_ADVICE[report.os] ?? GIT_ADVICE.linux!;

  return (
    <section aria-label="Getting started" className="mt-6 text-left">
      <ol className="divide-y divide-line rounded-md border border-line">
        <Step
          ok={report.git.path !== null}
          title={report.git.path ? "git is installed" : "git is not installed"}
          detail={report.git.version ?? undefined}
        >
          {!report.git.path && (
            <>
              <p>
                Yardsort keeps every workspace in a git worktree, so it cannot do anything without
                git. {gitAdvice.text}
              </p>
              {gitAdvice.command && <Command text={gitAdvice.command} />}
              <Link url={gitAdvice.url}>Other ways to install git</Link>
            </>
          )}
        </Step>

        <Step
          ok={installed.length > 0}
          title={
            installed.length > 0
              ? `${installed.length === 1 ? "1 coding agent" : `${installed.length} coding agents`} found`
              : "No coding agent found"
          }
          detail={installed.map((harness) => harness.label).join(", ") || undefined}
        >
          {installed.length === 0 && (
            <>
              <p>
                Yardsort runs agents you install yourself — it bundles none and never sees their
                credentials. Install any one of these, make sure it works in a terminal, then check
                again.
              </p>
              <ul className="mt-2 grid gap-3">
                {agents
                  .filter((harness) => harness.install)
                  .map((harness) => (
                    <li key={harness.id}>
                      <div className="flex items-center gap-2 font-medium text-ink">
                        <HarnessIcon id={harness.id} label={harness.label} />
                        {harness.label}
                      </div>
                      <Command text={harness.install!.command} />
                      <Link url={harness.install!.url}>Install instructions</Link>
                    </li>
                  ))}
              </ul>
              <p className="mt-3 text-ink-faint">
                Using something else? Add it under Settings → Harnesses.
              </p>
            </>
          )}
        </Step>

        {report.env.warning && (
          <Step
            ok={false}
            title="Your shell environment could not be read"
            detail={report.env.shell}
          >
            <p>
              Yardsort asks your login shell for its environment so that tools installed by mise,
              nvm, Homebrew or cargo are found. That failed, so it is using the environment it was
              started with, and installed tools may appear missing.
            </p>
            <p className="mt-1 font-mono text-[12px] text-ink-faint select-text">
              {report.env.warning}
            </p>
          </Step>
        )}

        <Step
          ok={ysIsCurrent(report.ys) ? true : null}
          title={ysSummary(report.ys)}
          detail={ysIsCurrent(report.ys) ? report.ys.version : "optional"}
        >
          {!ysIsCurrent(report.ys) && <YsCommand ys={report.ys} />}
        </Step>

        {report.ready && !hasProjects && (
          <Step ok={null} title="Add your first project">
            <p>Open a git repository on this computer, then start a workspace in it.</p>
            <button
              type="button"
              onClick={() => void openProjectFromDisk()}
              className="mt-2 rounded bg-accent px-4 py-1.5 font-medium text-canvas"
            >
              Open a folder…
            </button>
          </Step>
        )}
      </ol>

      {error && (
        <p role="alert" className="mt-3 text-red-400 select-text">
          {error}
        </p>
      )}
      {!report.ready && (
        <div className="mt-4 flex items-center justify-center gap-3">
          <button
            type="button"
            disabled={checking}
            onClick={recheck}
            className="rounded border border-line px-4 py-1.5 text-ink-muted hover:border-accent hover:text-ink disabled:opacity-40"
          >
            {checking ? "Checking…" : "Check again"}
          </button>
          <span className="text-ink-faint">after installing — no restart needed</span>
        </div>
      )}
    </section>
  );
}

function Step(props: {
  /** `true` done, `false` needs attention, `null` the next thing to do. */
  ok: boolean | null;
  title: string;
  detail?: string;
  children?: React.ReactNode;
}) {
  const mark = props.ok === true ? "✓" : props.ok === false ? "!" : "→";
  const tone =
    props.ok === true ? "text-green-400" : props.ok === false ? "text-red-400" : "text-accent";
  return (
    <li className="flex gap-3 px-4 py-3">
      <span aria-hidden className={`w-4 shrink-0 text-center font-semibold ${tone}`}>
        {mark}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="font-medium">{props.title}</span>
          {props.detail && (
            <span className="truncate text-[12px] text-ink-faint">{props.detail}</span>
          )}
        </div>
        {props.children && <div className="mt-1 text-ink-muted">{props.children}</div>}
      </div>
    </li>
  );
}

function Command({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () =>
    void writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }, console.error);
  return (
    <div className="mt-1 flex items-center gap-2 rounded bg-raised px-2 py-1">
      <code className="min-w-0 flex-1 overflow-x-auto font-mono text-[12px] whitespace-nowrap text-ink select-text">
        {text}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label={`Copy: ${text}`}
        className="shrink-0 rounded px-2 text-[11px] text-ink-faint hover:bg-line hover:text-ink"
      >
        {copied ? "copied" : "copy"}
      </button>
    </div>
  );
}

function Link({ url, children }: { url: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={() => void openUrl(url).catch(console.error)}
      title={url}
      className="mt-1 text-[12px] text-accent hover:underline"
    >
      {children} ↗
    </button>
  );
}
