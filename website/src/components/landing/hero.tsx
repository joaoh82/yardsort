import { CopyCommand } from "@/components/copy-command";
import { DownloadLabel } from "@/components/platform";
import { Shot } from "@/components/shot";
import { appVersion, BREW_COMMAND, RELEASES_URL } from "@/lib/site";

export function Hero() {
  return (
    <section
      aria-labelledby="h1"
      className="relative mx-auto max-w-[1120px] overflow-hidden px-5 pt-[72px] text-center md:px-8"
    >
      <p className="mb-5 font-mono text-[12.5px] tracking-[0.02em] text-muted">
        v{appVersion()} · Linux · macOS · Windows · GPL-3.0
      </p>
      <h1
        id="h1"
        className="mx-auto max-w-[860px] text-4xl leading-[1.05] font-medium tracking-[-0.02em] md:text-[60px] md:tracking-[-0.028em]"
      >
        Run AI coding agents in parallel — each on its own track.
      </h1>
      <p className="mx-auto mt-[22px] max-w-[640px] text-[17px] text-muted md:text-[19px]">
        A desktop app for Linux, macOS and Windows that gives every task its own git worktree and
        its own terminal, running the coding agent of your choice.
      </p>
      <div className="mx-auto mt-8 flex flex-col flex-wrap items-stretch justify-center gap-3 md:flex-row md:items-center">
        <a
          href={RELEASES_URL}
          className="rounded-md bg-accent px-[18px] py-[11px] text-[15px] font-medium text-accent-ink hover:text-accent-ink hover:opacity-90"
        >
          <DownloadLabel />
        </a>
        <CopyCommand command={BREW_COMMAND} size="hero" />
      </div>
      <p className="mt-3.5 text-[13px] text-muted">
        <a href="#install" className="link">
          All downloads
        </a>{" "}
        · needs git and one agent CLI that already works in your terminal
      </p>
      <div className="relative mx-auto mt-14 max-w-[1120px]">
        <div aria-hidden="true" className="track-band absolute -inset-x-8 -top-[72px] h-[120px]" />
        <Shot
          name="overview"
          alt="Yardsort: projects and workspaces on the left, an agent's terminal in the middle, its changes and a diff on the right"
          priority
          className="relative shadow-[0_30px_80px_-30px_var(--ys-shadow)]"
        />
      </div>
    </section>
  );
}
