import { Shot } from "@/components/shot";

const POINTS = [
  {
    title: "Private by construction",
    body: "No account, no telemetry, no keys of ours. Agents use their own logins; Yardsort just starts them. It checks for new versions, which you can switch off — and nothing else leaves your machine unless you switch Assist on and bring your own TypeSafe key.",
  },
  {
    title: "A record of what ran",
    body: "A local note of when each agent started in a workspace and how it ended — even while the window was closed — without reading a word it printed. Opt in, and every built-in agent reports its own tool calls and turns to it as metadata, its own settings untouched. An experimental timeline shows it; ys activity export writes it out.",
  },
  {
    title: "Closing the window doesn't stop them",
    body: "Terminals live in a small background process, so agents keep working while Yardsort is closed — or after it crashes. Open it again and every screen is repainted where it got to.",
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

export function Also() {
  return (
    <section aria-labelledby="h-also" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <div className="grid items-start gap-12 md:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <div className="md:order-2">
          <p className="mb-2.5 font-mono text-[12.5px] text-accent">08 · Also</p>
          <h3 id="h-also" className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">
            Quiet about the rest
          </h3>
          <dl className="mt-6 grid gap-[18px] text-[15px]">
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
