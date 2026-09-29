import { useEffect, useState } from "react";
import { errorMessage, ipc, type DraftStatus } from "@/lib/ipc";
import { useDraftStore } from "@/stores/draft";
import { useHarnessStore } from "@/stores/harnesses";
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
  const harnesses = useHarnessStore((s) => s.harnesses);
  const workflowWriter = useDraftStore((s) => s.workflowWriter);
  const [key, setKey] = useState("");
  const [model, setModel] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    void useDraftStore.getState().load();
    void useHarnessStore.getState().load();
    void useDraftStore.getState().loadWorkflowWriter();
  }, []);
  if (!status) return null;

  const run = async (action: () => Promise<DraftStatus>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      useDraftStore.setState({ status: await action() });
      await useDraftStore.getState().loadWorkflowWriter();
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
        Writing commit messages, pull requests and workflows
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
            what comes back goes in the box for you to read and edit — never straight to git. Also
            enables Write it when describing a new workflow.
          </span>
        </span>
      </label>

      <p className="text-ink-faint">
        {status.harness ? (
          <>
            <span className="text-ink-muted">{status.harness}</span> writes commit and pull request
            drafts, in its non-interactive mode — the agent you already have, billed to the account
            it already uses. An API key below is the fallback for when no agent here can.
          </>
        ) : (
          <>
            No configured agent can write one. Give a harness its non-interactive arguments under{" "}
            <span className="text-ink-muted">Harnesses → Write args</span>, or add a key below.
          </>
        )}
      </p>

      <Field
        label="Workflow writer"
        hint="Choose the agent that writes a new workflow from your description. Automatic uses the first installed, enabled harness with Write args, then the API key if none can write. Commit and PR drafts still prefer the workspace’s agent."
      >
        <select
          className={inputClass}
          value={workflowWriter?.harnessId ?? ""}
          disabled={busy || !workflowWriter}
          onChange={(event) => {
            const id = event.target.value || null;
            setBusy(true);
            setError(null);
            void ipc
              .workflowSaveWriter(id)
              .then(
                (writer) => useDraftStore.setState({ workflowWriter: writer }),
                (failed) => {
                  setError(errorMessage(failed));
                },
              )
              .finally(() => setBusy(false));
          }}
        >
          <option value="">Automatic</option>
          {workflowWriter?.harnessId &&
            !harnesses.some((h) => h.id === workflowWriter.harnessId) && (
              <option value={workflowWriter.harnessId} disabled>
                {workflowWriter.harnessId} (unavailable)
              </option>
            )}
          {harnesses.map((h) => (
            <option
              key={h.id}
              value={h.id}
              disabled={!h.enabled || !h.resolvedPath || !h.writeArgs?.length}
            >
              {h.label}
              {!h.enabled
                ? " (disabled)"
                : !h.resolvedPath
                  ? " (not installed)"
                  : !h.writeArgs?.length
                    ? " (needs Write args)"
                    : ""}
            </option>
          ))}
        </select>
      </Field>
      {workflowWriter?.status.problem && (
        <p className="text-ink-faint">{workflowWriter.status.problem}</p>
      )}

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
