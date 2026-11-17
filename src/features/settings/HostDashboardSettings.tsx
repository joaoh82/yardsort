import { useEffect, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { errorMessage, ipc, type DashboardSignIn, type DashboardStatus } from "@/lib/ipc";
import { buttonClass, primaryButtonClass } from "./fields";

/** How often to ask how the dashboard stands: who signed in, whether its tunnel came up. */
const POLL_MS = 5_000;

/**
 * The host dashboard: a web page this computer serves to itself, for looking after it from a
 * browser. Off until turned on here. A browser gets in only with a one-time link, and the only
 * place one is made is this window, which is where a person is.
 */
export function HostDashboardSettings() {
  const [status, setStatus] = useState<DashboardStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [copied, setCopied] = useState<DashboardSignIn | null>(null);

  useEffect(() => {
    let stale = false;
    const read = () =>
      ipc.dashboardStatus().then(
        (next) => {
          if (!stale) setStatus(next);
        },
        () => {},
      );
    void read();
    const timer = setInterval(() => void read(), POLL_MS);
    return () => {
      stale = true;
      clearInterval(timer);
    };
  }, []);

  if (!status) return null;
  const run = async (action: () => Promise<DashboardStatus | void>) => {
    setBusy(true);
    setProblem(null);
    setCopied(null);
    try {
      const next = await action();
      if (next) setStatus(next);
    } catch (error) {
      setProblem(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };
  const copy = () =>
    run(async () => {
      const signIn = await ipc.dashboardSignInLink();
      await writeText(signIn.link);
      setCopied(signIn);
    });

  return (
    <section aria-label="Host dashboard" className="grid gap-2">
      <h4 className="text-ink-muted">Host dashboard</h4>
      <p className="text-ink-faint">
        A web page this computer serves to itself, for looking after it from a browser: how it
        stands, the agents installed, Git and GitHub, and commands that set it up. It is not another
        Yardsort, and needs no Remote. It listens on this computer alone, and a browser signs in
        with a one-time link made here.
      </p>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={status.enabled}
          disabled={busy}
          onChange={(event) => {
            const enabled = event.target.checked;
            void run(() => ipc.dashboardSetEnabled(enabled));
          }}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Serve the host dashboard
          <span className="block text-ink-faint">
            Off unless you turn it on. Whoever signs in to it can run commands on this computer, as
            you.
          </span>
        </span>
      </label>
      {status.address && (
        <>
          <p className="text-ink-faint">
            Served at <span className="font-mono text-ink-muted select-text">{status.address}</span>
            . {signedInText(status.signedIn)}
            {status.tunnel && ` Its tunnel is ${status.tunnel}.`}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(() => ipc.dashboardOpen())}
              className={primaryButtonClass}
            >
              Open the dashboard
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void copy()}
              className={buttonClass}
            >
              Copy a sign-in link
            </button>
            <button
              type="button"
              disabled={busy || status.signedIn === 0}
              onClick={() => void run(() => ipc.dashboardSignEveryoneOut())}
              className={buttonClass}
            >
              Sign every browser out
            </button>
          </div>
          {copied && (
            <p role="status" className="text-ink-faint">
              Copied. It signs one browser in, once, within {copied.goodForMinutes} minutes.
              {onThisComputerOnly(copied.link) &&
                " It is this computer's own address: from another, forward the port over SSH first, or turn on the tunnel in the dashboard, under Access."}
            </p>
          )}
        </>
      )}
      {status.problem && (
        <p role="alert" className="text-red-400 select-text">
          The dashboard is turned on and not served: {status.problem}
        </p>
      )}
      {problem && (
        <p role="alert" className="text-red-400 select-text">
          {problem}
        </p>
      )}
    </section>
  );
}

function signedInText(count: number): string {
  if (count === 0) return "No browser is signed in.";
  if (count === 1) return "One browser is signed in.";
  return `${count} browsers are signed in.`;
}

/** Whether a link leads to this computer's loopback address, which no other computer reaches. */
function onThisComputerOnly(link: string): boolean {
  return link.startsWith("http://127.0.0.1:");
}
