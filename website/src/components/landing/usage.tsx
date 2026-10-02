import Link from "next/link";
import { Shot } from "@/components/shot";

export function Usage() {
  return (
    <section
      id="usage"
      aria-labelledby="h-usage"
      className="mx-auto max-w-[1120px] px-5 pt-[120px] md:px-8"
    >
      <p className="mb-2.5 font-mono text-[12.5px] text-accent">Usage</p>
      <h2
        id="h-usage"
        className="max-w-[720px] text-[26px] leading-[1.15] font-medium tracking-[-0.02em] md:text-[32px]"
      >
        See the tokens spent and the machine doing the work
      </h2>
      <p className="mt-4 max-w-[720px] text-[17px] text-muted">
        Open Usage beside Settings in the sidebar, or from the command palette. Two tabs help you
        compare your agents and find which workspace is using CPU or memory. All figures stay on
        your machine, with no extra API key or activity capture needed.
      </p>
      <div className="mt-8 grid gap-8 md:grid-cols-2">
        <div className="border-t border-line pt-5">
          <h3 className="text-xl font-medium">Token usage</h3>
          <Shot
            name="usage-tokens"
            alt="Token usage with demo logs: daily costs, cache savings and workspace totals"
            className="mt-4"
          />
          <p className="mt-3 text-muted">
            Compare Claude Code, Codex and Grok over 7, 30 or 90 days, by day, agent, model and
            workspace. Yardsort reads their local session logs, including conversations started
            outside the app. Inspect cache reads, writes and savings, and Codex&apos;s plan limits
            as it last reported them.
          </p>
          <p className="mt-3 text-muted">
            Switch between tokens and estimated API cost in US dollars. Cost is a comparison, rather
            than your subscription bill; unknown models, including Grok models, count toward tokens
            but have no cost estimate.
          </p>
        </div>
        <div className="border-t border-line pt-5">
          <h3 className="text-xl font-medium">Machine resources</h3>
          <Shot
            name="usage-machine"
            alt="Machine resources: live CPU, memory and terminal process trees in demo projects"
            className="mt-4"
          />
          <p className="mt-3 text-muted">
            Follow live CPU and memory for Yardsort, its terminal host and every terminal it runs.
            Each terminal includes its agent and the programs it starts — builds, tests and language
            servers alike. This tab covers all agents, shells and run commands.
          </p>
          <p className="mt-3 text-muted">
            Sort by CPU or memory, expand projects and workspaces, and click a workspace to return
            to its terminal. Five-minute charts show recent samples; sampling runs every two seconds
            while this tab is open and the window is visible.
          </p>
        </div>
      </div>
      <p className="mt-6 text-sm text-muted">
        Token history covers logs still present on this computer. Neither tab includes other
        machines.{" "}
        <Link href="/docs/guide/usage/" className="link">
          Read the Usage guide →
        </Link>
      </p>
    </section>
  );
}
