import { Shot } from "@/components/shot";

const POINTS = [
  {
    title: "Private by construction",
    body: "No account, no telemetry, no keys. Agents use their own logins; Yardsort just starts them. Its one network request is a check for new versions, which you can switch off.",
  },
  {
    title: "Careful with your work",
    body: "Deleting or archiving a workspace always keeps the branch, and never discards uncommitted changes without a second, explicit confirmation.",
  },
  {
    title: "Keeps itself current",
    body: "Signed in-app updates on macOS, Windows and the Linux AppImage — one click, and your agents' conversations resume afterwards.",
  },
  {
    title: "Light",
    body: "Built with Tauri and Rust: a few megabytes, not a bundled browser.",
  },
];

export function Harnesses() {
  return (
    <section aria-labelledby="h-also" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <div className="grid items-start gap-12 md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <div>
          <p className="mb-2.5 font-mono text-[12.5px] text-accent">05 · Harnesses</p>
          <h3 id="h-also" className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">
            Any terminal agent
          </h3>
          <p className="mt-3.5 text-muted">
            Claude Code, Codex, Grok and OpenCode out of the box; add any other with a few lines of
            configuration — no plugin, no release to wait for. To Yardsort a harness is a command
            and some argument templates, kept in a plain TOML file you can read, back up and edit.
          </p>
          <dl className="mt-8 grid gap-[18px] text-[15px]">
            {POINTS.map((point) => (
              <div key={point.title}>
                <dt className="font-medium">{point.title}</dt>
                <dd className="mt-0.5 text-muted">{point.body}</dd>
              </div>
            ))}
          </dl>
        </div>
        <Shot
          name="settings"
          alt="Harness settings: label, command and argument templates for Claude Code"
        />
      </div>
    </section>
  );
}
