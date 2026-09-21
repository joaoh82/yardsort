import type { ReactNode } from "react";
import { Shot } from "@/components/shot";

const code = "font-mono text-[13.5px] text-ink";
const strong = "font-medium text-ink";

type IdeaProps = {
  label: string;
  title: string;
  shot: ReactNode;
  // On wide screens the screenshot alternates sides; on narrow ones the text always comes first.
  shotFirst?: boolean;
  children: ReactNode;
};

export function Idea({ label, title, shot, shotFirst, children }: IdeaProps) {
  return (
    <div
      className={`grid items-center gap-12 ${
        shotFirst
          ? "md:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]"
          : "md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]"
      }`}
    >
      <div className={shotFirst ? "md:order-2" : undefined}>
        <p className="mb-2.5 font-mono text-[12.5px] text-accent">{label}</p>
        <h3 className="text-2xl leading-[1.2] font-medium tracking-[-0.015em]">{title}</h3>
        <div className="mt-3.5 space-y-3 text-muted">{children}</div>
      </div>
      {shot}
    </div>
  );
}

export function Ideas() {
  return (
    <section aria-labelledby="h-how" className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8">
      <h2
        id="h-how"
        className="max-w-[720px] text-[26px] leading-[1.15] font-medium tracking-[-0.02em] md:text-[32px]"
      >
        A sorting yard is where rail cars are sorted onto parallel tracks and later joined back into
        one train.
      </h2>
      <p className="mt-4 max-w-[640px] text-[17px] text-muted">
        That is the job: fan work out onto parallel branches, then merge it back.
      </p>

      <div className="mt-[72px] space-y-[88px]">
        <Idea
          label="01 · Workspaces"
          title="Every task gets its own worktree"
          shot={
            <Shot
              name="composer"
              alt="The composer: describe a task, pick an agent, model, effort and branch"
            />
          }
        >
          <p>
            You describe a task, pick an agent, and press Enter. Yardsort creates a branch and a git
            worktree for it — a separate folder — and starts the agent there. Start another, and
            another. They cannot disturb each other, or your own checkout.
          </p>
          <p>
            Workspaces are ordinary worktrees and branches. Inspect or undo anything with{" "}
            <code className={code}>git</code>. Worktrees made elsewhere are picked up automatically.
          </p>
        </Idea>

        <Idea
          shotFirst
          label="02 · Terminal"
          title="The terminal is the truth"
          shot={
            <Shot
              name="overview"
              crop="45% 60%"
              alt="An agent's own terminal interface running inside Yardsort"
            />
          }
        >
          <p>
            Agents run in a real PTY with their own interface. Whatever they can do in your
            terminal, they can do here — and Yardsort never parses their output.
          </p>
          <p>
            Status dots show which agents are working and which are waiting; a desktop notification
            tells you when one finishes while you are elsewhere. A shell tab next to the agent is
            one <code className={code}>Ctrl+Shift+T</code> away.
          </p>
        </Idea>

        <Idea
          label="03 · Changes"
          title="See what happened before anything merges"
          shot={
            <Shot
              name="overview"
              crop="100% 50%"
              alt="The changes panel: modified files with line counts, and a diff of render.js"
            />
          }
        >
          <p>
            A live list of changed files, character-level diffs, a file tree, and one click into
            your editor. Reviewing is read-only: Yardsort shows you changes; it does not stage,
            commit or discard them.
          </p>
          <p>
            When the work is done, it is an ordinary git branch — review it, push it, open a pull
            request.
          </p>
        </Idea>

        <Idea
          shotFirst
          label="04 · Sessions"
          title="Pick up where you left off"
          shot={<Shot name="sessions" alt="Previous sessions with Resume and Fork" />}
        >
          <p>
            Close Yardsort and your agents carry on: the terminals belong to a small background
            process, not to the window. Open it again and every screen is repainted where it got to.
            Closing with work in flight asks first.
          </p>
          <p>
            For a conversation that really did end, press <strong className={strong}>Resume</strong>{" "}
            — it is intact. <strong className={strong}>Fork</strong> one to try a different approach
            without losing the first.
          </p>
        </Idea>
      </div>
    </section>
  );
}
