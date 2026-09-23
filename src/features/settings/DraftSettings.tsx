import { useEffect, useState } from "react";
import { errorMessage, ipc, type DraftStatus } from "@/lib/ipc";
import { useDraftStore } from "@/stores/draft";
import { buttonClass, Field, inputClass } from "./fields";

/**
 * Writing with a model: the ✦ button beside the commit box and in the pull request dialog.
 *
 * It sits under Assist because that is where "a model does something optional for you" lives,
 * but it is not Assist: Jev answers typed questions and never writes text, so this is the one
 * place Yardsort asks a model to produce something. See `crate::draft`.
 */
export function DraftSettings() {
  const status = useDraftStore((s) => s.status);
  const [key, setKey] = useState("");
  const [model, setModel] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => void useDraftStore.getState().load(), []);
  if (!status) return null;

  const run = async (action: () => Promise<DraftStatus>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      useDraftStore.setState({ status: await action() });
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const wanted = model ?? status.model;
  return (
    <fieldset className="grid gap-3">
      <legend className="mb-1 text-[11px] font-semibold tracking-wider text-ink-muted uppercase">
        Writing commit messages and pull requests
      </legend>

      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          checked={status.enabled}
          disabled={busy}
          onChange={(event) => void run(() => ipc.draftSaveSettings(event.target.checked, wanted))}
          className="mt-0.5 accent-(--color-accent)"
        />
        <span>
          Offer to write them for me
          <span className="block text-ink-faint">
            A ✦ beside the commit box and in the pull request dialog. It sends that change&rsquo;s
            diff, and what the workspace was asked to do. Nothing happens until you press it, and
            what comes back goes in the box for you to read and edit — never straight to git.
          </span>
        </span>
      </label>

      <p className="text-ink-faint">
        {status.harness ? (
          <>
            <span className="text-ink-muted">{status.harness}</span> does the writing, in its
            non-interactive mode — the agent you already have, billed to the account it already
            uses. An API key below is the fallback for when no agent here can.
          </>
        ) : (
          <>
            No configured agent can write one. Give a harness its non-interactive arguments under{" "}
            <span className="text-ink-muted">Harnesses → Write args</span>, or add a key below.
          </>
        )}
      </p>

      <form
        onSubmit={(event) => {
          event.preventDefault();
          void run(() => ipc.draftSaveKey(key)).then((saved) => {
            if (saved) {
              setKey("");
              setNote("Key saved.");
            }
          });
        }}
      >
        <Field
          label="Anthropic API key"
          hint={
            status.key ? (
              <>
                In force. Used only when no agent can write, and asked for{" "}
                <code>{status.model}</code>.
              </>
            ) : (
              <>
                Optional. Kept in your system credential store, never in the settings file;{" "}
                <code>ANTHROPIC_API_KEY</code> from your environment works too.
              </>
            )
          }
          trailing={
            <>
              <button type="submit" disabled={!key.trim() || busy} className={buttonClass}>
                Save
              </button>
              {status.key && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run(() => ipc.draftForgetKey())}
                  className={buttonClass}
                >
                  Forget
                </button>
              )}
            </>
          }
        >
          <input
            type="password"
            value={key}
            placeholder={status.key ? "replace the key…" : "paste your key"}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setKey(event.target.value)}
            className={`${inputClass} font-mono text-[12px]`}
          />
        </Field>
      </form>

      <Field
        label="Model"
        hint="Asked for only when the API key is doing the writing. An agent uses whatever it is set to use."
        trailing={
          model !== null &&
          model.trim() !== status.model && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                void run(() => ipc.draftSaveSettings(status.enabled, wanted)).then((saved) => {
                  if (saved) setModel(null);
                })
              }
              className={buttonClass}
            >
              Save
            </button>
          )
        }
      >
        <input
          value={wanted}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => setModel(event.target.value)}
          className={`${inputClass} font-mono text-[12px]`}
        />
      </Field>

      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      {note && <p className="text-ink-faint">{note}</p>}
    </fieldset>
  );
}
