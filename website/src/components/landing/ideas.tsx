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
            <code className={code}>git</code>. Worktrees made elsewhere can be imported, and never
            appear uninvited.
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
            your editor.
          </p>
          <p>
            When it looks right, the foot of the same panel commits it, pushes it and opens the pull
            request — whose number and check results then sit on the workspace row. It stays an
            ordinary git branch throughout, and Yardsort holds no forge credentials: the{" "}
            <code className={code}>gh</code> you already use opens the pull request, or your browser
            does.
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

        <Idea
          label="05 · Handoff"
          title="Hand the work to another agent"
          shot={
            <Shot
              name="handoff"
              alt="Hand off: the composer holding the next agent's first message, written from what Yardsort recorded"
            />
          }
        >
          <p>
            One agent has worked in a workspace and you want another there — Codex after Claude, or
            the same agent from a clean start. The files and the diff carry over by themselves. What
            was asked, tried and found does not.
          </p>
          <p>
            <strong className={strong}>Hand off…</strong> writes the next agent&apos;s first message
            from what Yardsort recorded: your task, word for word; the branch and its commits; each
            changed file and who wrote it; what every agent run did — its tools, what failed, what
            it wrote; and what is not known. It opens in the composer, where you read it, edit it,
            pick the agent and press Enter.
          </p>
          <p>
            None of it is the last agent&apos;s words — Yardsort never keeps a conversation — and no
            model writes it. It says where it is blind, and tells the next agent to ask you. From a
            terminal, <code className={code}>ys workspace handoff</code> prints the same message.
          </p>
        </Idea>

        <Idea
          shotFirst
          label="06 · Workflows"
          title="Named agent work, in steps"
          shot={
            <Shot
              name="workflows"
              alt="Workflows: the built-in code review as a chart, with its file and its runs beside it"
            />
          }
        >
          <p>
            A workflow is a short YAML file: the inputs to ask for, and steps that start an agent,
            wait for it to settle, type to it once it is quiet, wait for a review on the pull
            request, and notify you. A step names the steps it needs, so the ones that need nothing
            of each other run side by side. Every name and every{" "}
            <code className={code}>{"{{ variable }}"}</code> is checked before anything runs, with
            the line and column of each mistake.
          </p>
          <p>
            <strong className={strong}>Workflows</strong>, above Projects, lists the built-in ones
            and yours: the steps as a chart, the file in an editor with every problem marked where
            it is, and the runs. Rather not write the file? Say what it should do and press{" "}
            <strong className={strong}>Write it</strong> — the agent you already have, or your
            Anthropic key, writes it, checked like any other and never saved until you say so.
          </p>
          <p>
            The built-in <strong className={strong}>Request code review</strong> is in every
            workspace&apos;s menu: a second agent reviews the pull request in the same worktree and
            posts on GitHub, then you are told, and the agent that wrote it is told to address the
            review. <code className={code}>ys workflow run</code> does the same from a terminal, and
            the open app carries it out in the background.
          </p>
        </Idea>
      </div>
    </section>
  );
}
