import { useEffect, useState } from "react";
import { ipc, type OtherHost } from "@/lib/ipc";
import { useOtherComputersStore } from "@/stores/otherComputers";
import { buttonClass, Field, inputClass, primaryButtonClass } from "./fields";
import { pairingText } from "@/features/sidebar/otherComputersText";

/**
 * Remote the other way round: this computer reaching other hosts, as a phone does. It signs in as a
 * device of its own, apart from this computer as a host above, and pairs by the code another
 * computer shows. That computer's person says what it may do, there.
 */
export function OtherComputersSettings() {
  const status = useOtherComputersStore((s) => s.status);
  const error = useOtherComputersStore((s) => s.error);
  const run = useOtherComputersStore((s) => s.run);
  // `null` until typed in: until then, the server and name this computer is a host by.
  const [typedServer, setServer] = useState<string | null>(null);
  const [typedName, setName] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void useOtherComputersStore.getState().load();
  }, []);

  if (!status) return null;
  const server = typedServer ?? status.suggestedServerUrl ?? "";
  const name = typedName ?? status.suggestedName ?? "";
  const act = async (action: () => Promise<Awaited<ReturnType<typeof ipc.otherComputers>>>) => {
    setBusy(true);
    try {
      return await run(action);
    } finally {
      setBusy(false);
    }
  };
  const client = status.client;

  return (
    <section aria-label="Other computers" className="grid gap-3 border-t border-line pt-5">
      <h4 className="text-ink-muted">Other computers</h4>
      <p className="text-ink-faint">
        Reach another computer running Yardsort from this one: its workspaces, agents and activity,
        read-only for now. This computer signs in as a device of its own, apart from being a host
        above, and the other computer&apos;s person lets it in there.
      </p>

      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}

      {client ? (
        <>
          <p>
            Signed in as <span className="text-ink">{client.name}</span> at{" "}
            <span className="font-mono text-detail select-text">{client.serverUrl}</span>.
          </p>
          {status.signedOut && (
            <p role="alert" className="text-red-400 select-text">
              This computer was signed out at the server, and reaches no other computer now.
            </p>
          )}
          <div className="flex items-center gap-2">
            {status.signedOut && (
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  void act(() => ipc.otherComputersSignIn(client.serverUrl, client.name))
                }
                className={primaryButtonClass}
              >
                Sign in again
              </button>
            )}
            <button
              type="button"
              disabled={busy}
              onClick={() => void act(ipc.otherComputersSignOut)}
              className={buttonClass}
            >
              Sign out
            </button>
            {status.signingIn && <Waiting />}
          </div>

          {!status.signedOut && (
            <form
              className="grid max-w-xl gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                void act(() => ipc.otherComputersPair(code)).then((paired) => {
                  if (paired) setCode("");
                });
              }}
            >
              <Field
                label="Pairing code"
                hint="On the other computer: Pair a device, under its Settings → Remote, or yardsort-host pair on one with no screen."
              >
                <input
                  value={code}
                  onChange={(event) => setCode(event.target.value)}
                  placeholder="ABCD-EFGH"
                  disabled={busy}
                  spellCheck={false}
                  autoCapitalize="characters"
                  className={inputClass}
                />
              </Field>
              <div>
                <button
                  type="submit"
                  disabled={busy || !code.trim()}
                  className={primaryButtonClass}
                >
                  Pair
                </button>
              </div>
            </form>
          )}

          <div className="grid gap-2">
            <h5 className="text-ink-muted">Pairings</h5>
            {status.hosts.length === 0 ? (
              <p className="text-ink-faint">None yet.</p>
            ) : (
              <ul className="grid gap-1">
                {status.hosts.map((host) => (
                  <Host
                    key={host.grantId}
                    host={host}
                    busy={busy}
                    onForget={() => void act(() => ipc.otherComputersForget(host.grantId))}
                  />
                ))}
              </ul>
            )}
          </div>
        </>
      ) : (
        <form
          className="grid max-w-xl gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void act(() => ipc.otherComputersSignIn(server, name));
          }}
        >
          <Field label="Server" hint="The one the other computer is signed in to.">
            <input
              value={server}
              onChange={(event) => setServer(event.target.value)}
              placeholder="https://remote.example.com"
              disabled={busy}
              spellCheck={false}
              className={inputClass}
            />
          </Field>
          <Field label="Name this computer" hint="What the other computer will call it.">
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Laptop"
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
              Sign in to reach other computers
            </button>
            {(status.signingIn || busy) && <Waiting />}
          </div>
        </form>
      )}
    </section>
  );
}

function Waiting() {
  return (
    <>
      <span className="text-ink-faint">Waiting for the browser…</span>
      <button
        type="button"
        onClick={() => void useOtherComputersStore.getState().run(ipc.otherComputersCancel)}
        className={buttonClass}
      >
        Cancel
      </button>
    </>
  );
}

function Host(props: { host: OtherHost; busy: boolean; onForget: () => void }) {
  const { host, busy, onForget } = props;
  return (
    <li className="flex items-center gap-3 rounded border border-line px-3 py-2">
      <span className="text-ink">{host.name}</span>
      <span className="text-ink-faint">{pairingText(host)}</span>
      <span className="flex-1" />
      <button
        type="button"
        disabled={busy}
        onClick={onForget}
        aria-label={`Forget ${host.name}`}
        className={buttonClass}
      >
        Forget
      </button>
    </li>
  );
}
