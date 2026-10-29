import { useEffect, useState } from "react";
import {
  errorMessage,
  ipc,
  type RemoteDevice,
  type RemoteLink,
  type RemotePairing,
  type RemoteRequest,
  type RemoteStatus,
} from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { useRemoteStore } from "@/stores/remote";
import { buttonClass, Field, inputClass, primaryButtonClass } from "./fields";

/** How often the open section asks the server who is waiting. */
const POLL_MS = 3000;

/**
 * Remote, a preview: this computer as a host on a Yardsort server, and the devices let in.
 *
 * Shown only when the app was started with `YARDSORT_REMOTE_PREVIEW=1`. While Remote is on, the
 * app keeps a link to the server's relay, and paired devices reach this computer through it; there
 * is no phone app yet. Approving a device happens here and nowhere else — not in `ys`, not
 * anywhere an agent could reach.
 */
export function RemoteSettings() {
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [server, setServer] = useState("");
  const [name, setName] = useState("");
  const [pairing, setPairing] = useState<RemotePairing | null>(null);
  const [requests, setRequests] = useState<RemoteRequest[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Off, the form offers what Remote was last turned on with, unless something is typed already.
  const show = (next: RemoteStatus) => {
    setStatus(next);
    if (next.host) return;
    setServer((typed) => typed || (next.lastServerUrl ?? ""));
    setName((typed) => typed || (next.lastName ?? ""));
  };

  useEffect(() => {
    void ipc
      .remoteStatus()
      .then(show)
      .catch((reason) => setError(errorMessage(reason)));
  }, []);

  const on = Boolean(status?.host);
  useEffect(() => {
    if (!on) return;
    let live = true;
    const poll = () =>
      ipc
        .remoteRequests()
        .then((answer) => {
          if (!live) return;
          setError(null);
          setRequests(answer.requests);
          setStatus(answer.status);
          useRemoteStore.getState().settled(answer.requests);
        })
        .catch((reason) => live && setError(errorMessage(reason)));
    void poll();
    const timer = window.setInterval(() => void poll(), POLL_MS);
    // A device asking is shown at once, not at the next look.
    const told = ipc.onRemotePairingRequested(() => void poll());
    return () => {
      live = false;
      window.clearInterval(timer);
      void told.then((stop) => stop());
    };
  }, [on]);

  if (!status) {
    return error ? <p role="alert">{error}</p> : null;
  }

  const run = async (action: () => Promise<RemoteStatus>) => {
    setBusy(true);
    setError(null);
    try {
      show(await action());
    } catch (reason) {
      setError(errorMessage(reason));
      show(await ipc.remoteStatus().catch(() => status));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-5 p-5">
      <section className="grid gap-2">
        <h3 className="text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
          Remote <span className="font-normal normal-case">(preview)</span>
        </h3>
        <p className="text-ink-faint">
          Let your phone or another computer reach this one through a Yardsort server you sign in
          to. A device you pair asks first, and gets in only if you say yes here. Agents keep
          running on this computer whatever happens to the connection.
        </p>
        <p className="text-ink-faint">
          In this preview there is no phone app yet. Whoever runs the server can see what passes
          through it; it is not end-to-end encrypted.
        </p>
      </section>

      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}

      {status.host ? (
        <>
          <section aria-label="This computer" className="grid gap-2">
            <p>
              On, as <span className="text-ink">{status.host.name}</span> at{" "}
              <span className="font-mono text-[12px] select-text">{status.host.serverUrl}</span>.
            </p>
            <p className="text-ink-faint">Its sign-in is kept {status.credentials}.</p>
            <LinkLine link={status.host.link} />
            <div className="flex gap-2">
              <button
                type="button"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  setError(null);
                  try {
                    setPairing(await ipc.remotePair());
                  } catch (reason) {
                    setError(errorMessage(reason));
                  } finally {
                    setBusy(false);
                  }
                }}
                className={primaryButtonClass}
              >
                Pair a device
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setPairing(null);
                  void run(ipc.remoteDisable);
                }}
                className={buttonClass}
              >
                Turn off
              </button>
            </div>
          </section>

          {pairing && <Pairing pairing={pairing} onDone={() => setPairing(null)} />}

          {requests.length > 0 && (
            <section aria-label="Waiting for you" className="grid gap-2">
              <h4 className="text-ink-muted">Waiting for you</h4>
              {requests.map((request) => (
                <Request
                  key={request.grantId}
                  request={request}
                  busy={busy}
                  onAnswer={(answer) =>
                    void run(async () => {
                      const next = await ipc.remoteAnswer(request.grantId, answer);
                      setRequests((all) => all.filter((r) => r.grantId !== request.grantId));
                      setPairing(null);
                      return next;
                    })
                  }
                />
              ))}
            </section>
          )}

          <section aria-label="Devices let in" className="grid gap-2">
            <h4 className="text-ink-muted">Devices let in</h4>
            {status.devices.length === 0 ? (
              <p className="text-ink-faint">None yet.</p>
            ) : (
              <ul className="grid gap-1">
                {status.devices.map((device) => (
                  <Device
                    key={device.grantId}
                    device={device}
                    busy={busy}
                    onRevoke={() => void run(() => ipc.remoteRevoke(device.grantId))}
                  />
                ))}
              </ul>
            )}
          </section>
        </>
      ) : (
        <form
          className="grid max-w-xl gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void run(() => ipc.remoteEnable(server, name));
          }}
        >
          <Field label="Server" hint="Its address, as its operator gave it.">
            <input
              value={server}
              onChange={(event) => setServer(event.target.value)}
              placeholder="https://remote.example.com"
              disabled={busy}
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Name this computer" hint="What your devices will call it.">
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Workstation"
              disabled={busy}
              className={inputClass}
            />
          </Field>
          <div className="flex items-center gap-2">
            <button
              type="submit"
              disabled={busy || !server.trim() || !name.trim()}
              className={primaryButtonClass}
            >
              Turn on and sign in
            </button>
            {status.signingIn || busy ? (
              <>
                <span className="text-ink-faint">Waiting for the browser…</span>
                <button
                  type="button"
                  onClick={() => void ipc.remoteCancel()}
                  className={buttonClass}
                >
                  Cancel
                </button>
              </>
            ) : null}
          </div>
        </form>
      )}
    </div>
  );
}

/** The code to type and the QR code, with how long they are good for. */
function Pairing({ pairing, onDone }: { pairing: RemotePairing; onDone: () => void }) {
  const [left, setLeft] = useState(pairing.expiresInMs);
  useEffect(() => {
    const started = Date.now();
    const timer = window.setInterval(() => {
      setLeft(Math.max(0, pairing.expiresInMs - (Date.now() - started)));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [pairing]);
  const seconds = Math.ceil(left / 1000);
  return (
    <section aria-label="Pair a device" className="grid gap-3 rounded border border-line p-4">
      <div className="flex flex-wrap items-center gap-6">
        <img
          alt="QR code for pairing"
          src={`data:image/svg+xml;utf8,${encodeURIComponent(pairing.qrSvg)}`}
          className="h-40 w-40 rounded bg-white"
        />
        <div className="grid gap-2">
          <p className="text-ink-faint">On the device, scan the code, or type:</p>
          <p
            aria-label="Pairing code"
            className="font-mono text-2xl tracking-widest text-ink select-text"
          >
            {pairing.code}
          </p>
          <p className="text-ink-faint">
            {seconds > 0
              ? `Good once, for ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")} more. The device then waits here for your answer.`
              : "Expired. Show a new code."}
          </p>
        </div>
      </div>
      <div>
        <button type="button" onClick={onDone} className={buttonClass}>
          Done
        </button>
      </div>
    </section>
  );
}

/** A device asking to be let in, and the answers. */
function Request(props: {
  request: RemoteRequest;
  busy: boolean;
  onAnswer: (answer: Parameters<typeof ipc.remoteAnswer>[1]) => void;
}) {
  const { request, busy, onAnswer } = props;
  const projects = useProjectsStore((s) => s.projects);
  const [chosen, setChosen] = useState<string[] | null>(null);
  return (
    <div
      role="group"
      aria-label={`${request.deviceName} asks to be let in`}
      className="grid gap-2 rounded border border-accent/60 p-3"
    >
      <p>
        <span className="text-ink">{request.deviceName}</span> asks to{" "}
        {request.wantsControl ? "watch and type into" : "watch"} this computer&rsquo;s terminals,
        and to see its workspaces and activity.
      </p>
      {request.wantsControl && (
        <p className="text-ink-faint">
          Typing into a terminal is anything you could do at this computer yourself.
        </p>
      )}
      <fieldset className="grid gap-1">
        <legend className="text-ink-muted">Projects</legend>
        <label className="flex items-center gap-2">
          <input
            type="radio"
            checked={chosen === null}
            onChange={() => setChosen(null)}
            className="accent-(--color-accent)"
          />
          All of them, and ones added later
        </label>
        <label className="flex items-center gap-2">
          <input
            type="radio"
            checked={chosen !== null}
            onChange={() => setChosen([])}
            className="accent-(--color-accent)"
          />
          Only these:
        </label>
        {chosen !== null && (
          <div className="grid gap-1 pl-6">
            {projects.map((project) => (
              <label key={project.id} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={chosen.includes(project.id)}
                  onChange={(event) =>
                    setChosen((all) =>
                      event.target.checked
                        ? [...(all ?? []), project.id]
                        : (all ?? []).filter((id) => id !== project.id),
                    )
                  }
                  className="accent-(--color-accent)"
                />
                {project.name}
              </label>
            ))}
          </div>
        )}
      </fieldset>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          disabled={busy || (chosen !== null && chosen.length === 0)}
          onClick={() => onAnswer({ kind: "allow", control: false, projects: chosen })}
          className={buttonClass}
        >
          Let it watch
        </button>
        {request.wantsControl && (
          <button
            type="button"
            disabled={busy || (chosen !== null && chosen.length === 0)}
            onClick={() => onAnswer({ kind: "allow", control: true, projects: chosen })}
            className={buttonClass}
          >
            Let it watch and type
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={() => onAnswer({ kind: "deny" })}
          className={buttonClass}
        >
          Refuse
        </button>
      </div>
    </div>
  );
}

function Device(props: { device: RemoteDevice; busy: boolean; onRevoke: () => void }) {
  const { device, busy, onRevoke } = props;
  const count = device.projects?.length;
  return (
    <li className="flex items-center gap-3 rounded border border-line px-3 py-2">
      <span className="text-ink">{device.name}</span>
      <span className="text-ink-faint">
        {device.control ? "watches and types" : "watches"},{" "}
        {count === undefined ? "all projects" : count === 1 ? "1 project" : `${count} projects`}
      </span>
      <span className="flex-1" />
      <button
        type="button"
        disabled={busy}
        onClick={onRevoke}
        aria-label={`Revoke ${device.name}`}
        className={buttonClass}
      >
        Revoke
      </button>
    </li>
  );
}

/** Where the link to the relay stands, in a sentence. Nothing while the app keeps none. */
function LinkLine({ link }: { link: RemoteLink }) {
  switch (link.kind) {
    case "off":
      return null;
    case "connecting":
      return <p className="text-ink-faint">Connecting to the relay…</p>;
    case "connected":
      return (
        <p>
          Connected to the relay.{" "}
          {link.devices === 0
            ? "No device is connected now."
            : link.devices === 1
              ? "1 device is connected now."
              : `${link.devices} devices are connected now.`}
        </p>
      );
    case "waiting":
      return (
        <p role="status" className="text-amber-300 select-text">
          Not connected to the relay: {link.why} Trying again in{" "}
          {Math.max(1, Math.round(link.retryInMs / 1000))} s.
        </p>
      );
    case "stopped":
      return (
        <p role="alert" className="text-red-400 select-text">
          Not connected to the relay: {link.why} Turn Remote off, then on again.
        </p>
      );
  }
}
