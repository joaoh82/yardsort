import Link from "next/link";
import { HarnessIcon } from "@/components/landing/harness-icon";
import { docUrl } from "@/lib/docs";

// The built-in harnesses, in the order the app lists them (`builtin()` in crates/core/src/harness.rs),
// with the command each one runs.
const BUILTIN: { id: string; label: string; command: string }[] = [
  { id: "claude", label: "Claude Code", command: "claude" },
  { id: "codex", label: "Codex", command: "codex" },
  { id: "grok", label: "Grok", command: "grok" },
  { id: "opencode", label: "OpenCode", command: "opencode" },
  { id: "omp", label: "OMP", command: "omp" },
  { id: "cursor", label: "Cursor", command: "cursor-agent" },
  { id: "pi", label: "Pi", command: "pi" },
];

// A whole custom harness, as settings.toml stores it.
const CUSTOM_TOML = `[[harness]]
id = "aider"
label = "Aider"
command = "aider"
prompt_transport = "stdin"`;

const code = "font-mono text-[13.5px] text-ink";

function Picker() {
  return (
    <figure
      aria-label="The agents Yardsort can start"
      className="overflow-hidden rounded-[10px] border border-line bg-surface shadow-[0_24px_60px_-30px_var(--ys-shadow)]"
    >
      <div className="flex items-center gap-2.5 border-b border-line bg-raised px-4 py-3">
        <span aria-hidden className="h-2 w-2 rounded-full bg-accent" />
        <span className="truncate text-[14px]">fix the flaky login test</span>
        <span aria-hidden className="h-4 w-px animate-pulse bg-ink" />
      </div>

      <p className="px-4 pt-3.5 pb-1.5 font-mono text-[11.5px] tracking-[0.08em] text-faint uppercase">
        Agent
      </p>
      <ul className="px-2 pb-2 text-[14px]">
        {BUILTIN.map((harness, index) => {
          const chosen = index === 0;
          return (
            <li
              key={harness.id}
              className={`relative flex items-center gap-3 rounded-md px-2.5 py-2 ${
                chosen ? "bg-raised" : ""
              }`}
            >
              {chosen && (
                <span aria-hidden className="absolute inset-y-2 left-0 w-0.5 rounded bg-accent" />
              )}
              <HarnessIcon id={harness.id} label={harness.label} />
              <span className={chosen ? "font-medium" : undefined}>{harness.label}</span>
              <code className="ml-auto truncate font-mono text-[12px] text-faint">
                {harness.command}
              </code>
            </li>
          );
        })}
        <li className="mt-1.5 flex items-center gap-3 rounded-md border border-dashed border-line px-2.5 py-2 text-muted">
          <HarnessIcon id="aider" label="Aider" />
          <span>Aider</span>
          <span className="rounded border border-line px-1.5 font-mono text-[11px] text-faint">
            yours
          </span>
          <code className="ml-auto truncate font-mono text-[12px] text-faint">aider</code>
        </li>
      </ul>

      <div className="border-t border-line bg-raised px-4 py-3.5">
        <p className="font-mono text-[11.5px] text-faint">settings.toml</p>
        <pre className="mt-1.5 overflow-x-auto font-mono text-[12.5px] leading-[1.6] text-muted">
          {CUSTOM_TOML}
        </pre>
      </div>
    </figure>
  );
}

export function Harnesses() {
  return (
    <section
      aria-labelledby="h-harnesses"
      className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8"
    >
      <div className="grid items-center gap-12 md:grid-cols-[minmax(0,6fr)_minmax(0,5fr)]">
        <div>
          <p className="mb-2.5 font-mono text-[12.5px] text-accent">07 · Harnesses</p>
          <h3 id="h-harnesses" className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">
            Switch agents. Keep the workflow.
          </h3>
          <div className="mt-3.5 space-y-3 text-muted">
            <p>
              Claude Code, Codex, Grok, OpenCode, OMP, Cursor and Pi work out of the box. Pick a
              different one for each task — the worktree, the branch, the review of its changes and
              the pull request stay the same whichever you choose.
            </p>
            <p>
              Add any other terminal agent with a few lines of configuration — no plugin, no release
              to wait for. To Yardsort a harness is a command and some argument templates, kept in a
              plain TOML file you can read, back up and edit. Its program is started with its own
              arguments, never through a shell, and uses its own login.
            </p>
            <p>
              <Link href={`${docUrl("guide/settings")}/`} className="link text-ink">
                Settings &amp; harnesses
              </Link>{" "}
              has every field, and how to set up <code className={code}>--model</code>,{" "}
              <code className={code}>--resume</code> and the rest.
            </p>
          </div>
        </div>
        <Picker />
      </div>
    </section>
  );
}
