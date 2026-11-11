import { useEffect, useState } from "react";
import { errorMessage, ipc, type SettingsInfo } from "@/lib/ipc";
import { native } from "@/lib/native";
import { buttonClass, Field, inputClass, primaryButtonClass } from "./fields";

export function WorkspaceSettings() {
  const [info, setInfo] = useState<SettingsInfo | null>(null);
  const [root, setRoot] = useState("");
  const [prefix, setPrefix] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const adopt = (next: SettingsInfo) => {
    setInfo(next);
    setRoot(next.workspaces.worktreeRoot ?? "");
    setPrefix(next.workspaces.branchPrefix);
  };

  useEffect(() => {
    ipc.settingsGet().then(adopt, (reason) => setError(errorMessage(reason)));
  }, []);

  if (!info) return <p className="p-5 text-ink-faint">{error ?? "Loading…"}</p>;

  const dirty =
    root.trim() !== (info.workspaces.worktreeRoot ?? "") ||
    prefix.trim() !== info.workspaces.branchPrefix;
  const example = `${prefix.trim() ? `${prefix.trim()}/` : ""}fix-login-bug`;

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      adopt(
        await ipc.settingsSaveWorkspaces({
          worktreeRoot: root.trim() || null,
          branchPrefix: prefix,
        }),
      );
      setSaved(true);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <form aria-label="Workspace settings" onSubmit={save} className="grid max-w-2xl gap-4 p-5">
      {info.problem && (
        <p role="alert" className="rounded border border-red-400/40 p-3 text-red-400 select-text">
          {info.problem}
          <br />
          Defaults are in use. Saving will keep your file as <code>settings.toml.unreadable</code>.
        </p>
      )}
      <Field
        label="Worktree folder"
        trailing={
          <button
            type="button"
            className={buttonClass}
            onClick={async () => {
              const folder = await native.pickFolder(
                "Worktree folder",
                root || info.defaultWorktreeRoot,
              );
              if (folder) {
                setRoot(folder);
                setSaved(false);
              }
            }}
          >
            Browse…
          </button>
        }
        hint={
          info.worktreeRootOverride ? (
            <span className="text-accent">
              Overridden by YARDSORT_WORKTREE_ROOT for this run: {info.worktreeRootOverride}
            </span>
          ) : (
            <>
              New workspaces are created in{" "}
              <code>&lt;folder&gt;/&lt;project&gt;/&lt;workspace&gt;</code>. Existing ones stay
              where they are. Keep it short on Windows.
            </>
          )
        }
      >
        <input
          value={root}
          placeholder={info.defaultWorktreeRoot}
          spellCheck={false}
          onChange={(e) => {
            setRoot(e.target.value);
            setSaved(false);
          }}
          className={`${inputClass} font-mono text-detail`}
        />
      </Field>

      <Field
        label="Branch prefix"
        hint={
          <>
            New branches look like <code>{example}</code>. Leave empty for no prefix.
          </>
        }
      >
        <input
          value={prefix}
          spellCheck={false}
          onChange={(e) => {
            setPrefix(e.target.value);
            setSaved(false);
          }}
          className={`${inputClass} w-48 font-mono text-detail`}
        />
      </Field>

      {error && (
        <p role="alert" className="text-red-400 select-text">
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <button type="submit" disabled={!dirty} className={primaryButtonClass}>
          Save
        </button>
        {saved && !dirty && <span className="text-ink-faint">Saved.</span>}
      </div>
      <p className="text-small text-ink-faint select-text">Settings file: {info.filePath}</p>
    </form>
  );
}
