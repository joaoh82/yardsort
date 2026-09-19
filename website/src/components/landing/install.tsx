import type { ReactNode } from "react";
import { CopyCommand } from "@/components/copy-command";
import { Detected, type Platform } from "@/components/platform";
import { BREW_COMMAND, RELEASES_URL, repoFile } from "@/lib/site";

function Card({
  platform,
  name,
  children,
}: {
  platform: Platform;
  name: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-3.5 rounded-[10px] border border-line bg-surface p-[22px]">
      <div className="flex items-baseline justify-between">
        <h3 className="text-[17px] font-medium">{name}</h3>
        <Detected platform={platform} />
      </div>
      {children}
    </div>
  );
}

function Formats({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap gap-2 font-mono text-[13px]">{children}</div>;
}

function Format({ children }: { children: ReactNode }) {
  return (
    <a href={RELEASES_URL} className="rounded-[5px] border border-line bg-raised px-2.5 py-1.5">
      {children}
    </a>
  );
}

function Pending({ children }: { children: ReactNode }) {
  return (
    <p className="mt-auto border-t border-dashed border-line pt-2.5 text-[13px] text-faint">
      {children}
    </p>
  );
}

const mono = "font-mono text-[12.5px]";

export function Install() {
  return (
    <section
      id="install"
      aria-labelledby="h-install"
      className="mx-auto max-w-[1120px] scroll-mt-6 px-5 pt-[104px] md:px-8"
    >
      <div className="grid items-end gap-8 md:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
        <h2
          id="h-install"
          className="text-[26px] leading-[1.15] font-medium tracking-[-0.02em] md:text-[32px]"
        >
          Install
        </h2>
        <p className="max-w-[640px] text-muted">
          Download the latest build from the{" "}
          <a href={RELEASES_URL} className="link text-ink">
            Releases page
          </a>
          . You also need git and at least one agent CLI that already works in your terminal.
          Yardsort does not bundle agents and never sees their credentials.
        </p>
      </div>
      <div className="mt-8 grid gap-4 md:grid-cols-3">
        <Card platform="linux" name="Linux">
          <Formats>
            <Format>.AppImage</Format>
            <Format>.deb</Format>
            <Format>.rpm</Format>
          </Formats>
          <p className="text-sm text-muted">
            AppImage: <code className={mono}>chmod +x Yardsort_*.AppImage</code> and run it. The
            AppImage updates itself; .deb and .rpm are told when a new version exists.
          </p>
          <Pending>
            <code className={mono}>yardsort-bin</code> on the AUR — on its way. Until then the
            AppImage works on Arch.
          </Pending>
        </Card>
        <Card platform="mac" name="macOS">
          <Formats>
            <Format>.dmg</Format>
            <span className="py-1.5 font-sans text-[13px] text-muted">
              universal · signed and notarized
            </span>
          </Formats>
          <CopyCommand command={BREW_COMMAND} />
          <p className="mt-auto text-sm text-muted">
            Open the .dmg and drag Yardsort to Applications. The app updates itself.
          </p>
        </Card>
        <Card platform="win" name="Windows">
          <Formats>
            <Format>-setup.exe</Format>
            <Format>.msi</Format>
          </Formats>
          <p className="text-sm text-muted">
            Not code-signed yet, so SmartScreen warns: choose{" "}
            <strong className="font-medium text-ink">More info → Run anyway</strong>. Needs{" "}
            <a href="https://git-scm.com/download/win" className="link">
              Git for Windows
            </a>
            .
          </p>
          <Pending>
            <code className={mono}>winget install joaoh82.Yardsort</code> — awaiting
            Microsoft&apos;s review.
          </Pending>
        </Card>
      </div>
      <p className="mt-5 text-sm text-muted">
        Prefer to build it yourself?{" "}
        <code className="font-mono text-[13px] break-words text-ink">
          git clone https://github.com/joaoh82/yardsort &amp;&amp; cd yardsort &amp;&amp; just setup
          &amp;&amp; just build
        </code>{" "}
        — see{" "}
        <a href={repoFile("CONTRIBUTING.md")} className="link">
          CONTRIBUTING.md
        </a>
        .
      </p>
    </section>
  );
}
