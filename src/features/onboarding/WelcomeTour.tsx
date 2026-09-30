import { useEffect, useRef, useState } from "react";
import { useModalFocus } from "@/lib/useModalFocus";
import { useLayoutStore } from "@/stores/layout";
import { usePreferencesStore, useShortcutLabel } from "@/stores/preferences";
import { buttonClass, primaryButtonClass } from "@/features/settings/fields";

const STEPS = [
  {
    title: "Your projects live here",
    target: "Projects",
    text: "Add a folder containing a git repository. Each project groups its workspaces. The local workspace is your original checkout; new workspaces get their own branch and worktree, so agents can work in parallel.",
  },
  {
    title: "Give an agent a task",
    target: "Workspace",
    text: "Choose + beside a project to open the composer. Pick an installed agent, describe the task and start. This center panel becomes a real terminal: you can respond to the agent, open more tabs and return to past sessions.",
  },
  {
    title: "Review the work",
    target: "Changes",
    text: "After selecting a workspace, use Changes to inspect its diff or Files to browse and edit text and preview images. Review and commit the work here, then publish a pull request when you are ready.",
  },
  {
    title: "Repeat work with workflows",
    target: "Projects",
    text: "The Workflows section above Projects holds reusable sequences of agent work. Open one to inspect its steps, edit it or run it. You can come back to workflows after your first workspace.",
  },
  {
    title: "Make Yardsort yours",
    target: "help",
    text: "Settings configures your agents, workspaces and keyboard shortcuts. The command palette finds commands and workspaces. Help stays in the bottom bar so you can replay this tour any time.",
  },
] as const;

export function WelcomeTour() {
  const loaded = usePreferencesStore((s) => s.loaded);
  const seen = usePreferencesStore((s) => s.welcomeSeen);
  const replay = useLayoutStore((s) => s.tourOpen);
  // Wait for core preferences: a slow startup must not flash a first-run invitation.
  return replay || (loaded && !seen) ? <TourDialog replay={replay} /> : null;
}

function TourDialog({ replay }: { replay: boolean }) {
  const [step, setStep] = useState(replay ? 0 : -1);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(ref);
  const before = useRef(useLayoutStore.getState().collapsed);
  const saving = usePreferencesStore((s) => s.saving);
  const error = usePreferencesStore((s) => s.error);
  const paletteKey = useShortcutLabel("palette");
  const workspaceKey = useShortcutLabel("newWorkspace");
  const current = step >= 0 ? STEPS[step] : null;
  const close = async () => {
    if (
      usePreferencesStore.getState().loaded &&
      !usePreferencesStore.getState().welcomeSeen &&
      !(await usePreferencesStore.getState().dismissWelcome())
    )
      return;
    useLayoutStore.getState().setTourOpen(false);
  };
  const start = async () => {
    // Open the replay flag before recording acceptance so the tour stays mounted.
    useLayoutStore.getState().setTourOpen(true);
    if (await usePreferencesStore.getState().dismissWelcome()) setStep(0);
  };
  useEffect(() => {
    const original = before.current;
    return () => {
      const layout = useLayoutStore.getState();
      layout.setCollapsed("left", original.left);
      layout.setCollapsed("right", original.right);
    };
  }, []);
  useEffect(() => {
    if (!current) return;
    const layout = useLayoutStore.getState();
    if (current.target === "Projects") layout.setCollapsed("left", false);
    if (current.target === "Changes") layout.setCollapsed("right", false);
    const target = document.querySelector<HTMLElement>(`[data-navigation="${current.target}"]`);
    const measure = () => setRect(target?.getBoundingClientRect() ?? null);
    const frame = requestAnimationFrame(measure);
    const observer = new ResizeObserver(measure);
    if (target) observer.observe(target);
    window.addEventListener("resize", measure);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [current]);
  return (
    <div
      className={`fixed inset-0 z-50 flex p-6 ${current ? "items-end" : "items-center justify-center bg-black/60"} ${current ? (current.target === "Changes" ? "justify-start" : "justify-end") : ""}`}
    >
      {current && rect && rect.width > 0 ? (
        <div
          aria-hidden
          className="pointer-events-none fixed rounded border-2 border-accent shadow-[0_0_0_9999px_rgba(0,0,0,0.65)]"
          style={{
            top: rect.top + 2,
            left: rect.left + 2,
            width: Math.max(0, rect.width - 4),
            height: Math.max(0, rect.height - 4),
          }}
        />
      ) : current ? (
        <div className="absolute inset-0 bg-black/60" />
      ) : null}
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="tour-title"
        aria-describedby="tour-description"
        tabIndex={-1}
        className="relative mb-7 max-h-[85vh] w-full max-w-md overflow-y-auto rounded-xl border border-line bg-surface p-6 shadow-2xl outline-none"
        style={
          current && rect && (current.target === "Projects" || current.target === "Changes")
            ? {
                position: "fixed",
                left: Math.max(
                  24,
                  Math.min(
                    window.innerWidth - 472,
                    current.target === "Projects" ? rect.right + 16 : rect.left - 464,
                  ),
                ),
                top: 48,
                width: "calc(100vw - 48px)",
              }
            : undefined
        }
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            event.preventDefault();
            void close();
          }
          if (step >= 0 && (event.key === "ArrowRight" || event.key === "ArrowLeft")) {
            event.preventDefault();
            setStep(
              Math.max(0, Math.min(STEPS.length - 1, step + (event.key === "ArrowRight" ? 1 : -1))),
            );
          }
        }}
      >
        <p className="mb-3 text-[11px] font-medium tracking-widest text-accent uppercase">
          {current ? `Welcome tour · ${step + 1} of ${STEPS.length}` : "Welcome to Yardsort"}
        </p>
        <h2 id="tour-title" className="text-xl font-semibold">
          {current?.title ?? "Take a quick look around?"}
        </h2>
        <p id="tour-description" className="mt-3 leading-relaxed text-ink-muted">
          {current?.text ??
            "A short guided tour will show you the main parts of the app, from your first project to reviewing an agent’s work. You can skip it now and replay it later from Help."}
        </p>
        {step === 1 && <p className="mt-3 text-accent">New workspace: {workspaceKey}</p>}
        {step === 4 && <p className="mt-3 text-accent">Commands and workspaces: {paletteKey}</p>}
        {error && (
          <p role="alert" className="mt-3 text-red-400">
            {error}
          </p>
        )}
        {current && (
          <div className="mt-5 flex gap-1.5" aria-label={`Step ${step + 1} of ${STEPS.length}`}>
            {STEPS.map((s, i) => (
              <span
                key={s.title}
                className={`h-1 flex-1 rounded ${i <= step ? "bg-accent" : "bg-line"}`}
              />
            ))}
          </div>
        )}
        <div className="mt-6 flex items-center gap-2">
          <button
            type="button"
            className={buttonClass}
            disabled={saving}
            onClick={() => void close()}
          >
            {current ? "Skip tour" : "Not now"}
          </button>
          <span className="flex-1" />
          {step > 0 && (
            <button type="button" className={buttonClass} onClick={() => setStep(step - 1)}>
              Back
            </button>
          )}
          <button
            type="button"
            className={primaryButtonClass}
            disabled={saving}
            onClick={() => {
              if (!current) void start();
              else if (step === STEPS.length - 1) void close();
              else setStep(step + 1);
            }}
          >
            {!current ? "Take the tour" : step === STEPS.length - 1 ? "Finish" : "Next"}
          </button>
        </div>
        {current && (
          <p className="mt-3 text-[11px] text-ink-faint">
            ← → to explore · Tab to move between buttons · Esc to leave
          </p>
        )}
      </div>
    </div>
  );
}
