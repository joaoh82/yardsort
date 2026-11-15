import { useEffect, useId, useRef, useState } from "react";
import { errorMessage, ipc, type LocalRepository, type OtherBranches } from "@/lib/ipc";
import { HarnessIcon } from "@/features/harness/HarnessIcon";
import { linkText } from "@/features/sidebar/otherComputersText";
import { thereKey, useOtherComputersStore } from "@/stores/otherComputers";
import { recall, useProjectsStore } from "@/stores/projects";
import { RunOn } from "./RunOn";
import { offeredThere, PROJECT_THERE, startThere } from "./there";

const control =
  "h-9 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50";

/** The choice in the project list that has that computer clone this computer's project. */
const CLONE = "clone:";

/**
 * The start of a workspace on another computer: say what you want, pick which of that computer's
 * agents does it, press Enter. The workspace, its branch and its worktree are made there, as that
 * computer's own window makes them, and nothing is made on this computer. If starting fails, the
 * message stays, and the start is not tried again by itself.
 *
 * Its title says which computer, and is where another is chosen. Opened from one of this
 * computer's projects (`from`), the project there is the one offered for it, shown and changeable
 * before the start, and remembered once something starts. When that computer has no such project
 * and can clone, **Clone it there** has it clone the repository first, with its own Git sign-in.
 *
 * A computer new enough takes what the local composer says too: a name, a model, an effort, and
 * the branch to start from or to open, each from that computer's own lists. An older one decides
 * them itself, and the controls are not shown.
 */
export function RemoteComposer({
  hostId,
  projectId,
  from = null,
}: {
  hostId: string;
  projectId: string | null;
  from?: string | null;
}) {
  const view = useOtherComputersStore((s) => s.views[hostId]);
  const hostName = useOtherComputersStore(
    (s) => s.status?.hosts.find((h) => h.hostId === hostId)?.name,
  );
  const agents = useOtherComputersStore((s) => s.harnesses[hostId]);
  const problem = useOtherComputersStore((s) => s.startProblem);
  const local = useProjectsStore((s) => s.projects.find((candidate) => candidate.id === from));
  const localProjects = useProjectsStore((s) => s.projects);
  const ui = useProjectsStore((s) => s.ui);
  const [agent, setAgent] = useState<string | null>(null);
  // What was typed before the computer was chosen comes along. Read once: choosing another
  // computer is a new mount, and brings what is here by then.
  const [message, setMessage] = useState(
    () => useProjectsStore.getState().composingThere?.message ?? "",
  );
  const [busy, setBusy] = useState<"starting" | "cloning" | null>(null);
  const [workspaceName, setWorkspaceName] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  // "new:<branch>" starts a new branch from <branch>; "open:<branch>" opens <branch> itself.
  const [base, setBase] = useState("");
  const [branches, setBranches] = useState<{ of: string; list: OtherBranches } | null>(null);
  // Which repository this computer's project is, to offer the same one there and to clone it.
  const [repository, setRepository] = useState<LocalRepository | null>(null);
  const [cloneThere, setCloneThere] = useState(false);
  // A project that computer cloned a moment ago, until its next reading lists it.
  const [cloned, setCloned] = useState<{ id: string; name: string } | null>(null);
  // Once the person has chosen a project there, nothing offered replaces it.
  const [picked, setPicked] = useState(false);
  const projectFieldId = useId();
  const nameFieldId = useId();
  const modelsId = useId();
  const messageRef = useRef<HTMLTextAreaElement>(null);
  // Taken before anything is awaited: a second Start while the first is on its way would start
  // a second agent on the same task.
  const starting = useRef(false);

  const name = view?.name || hostName || "another computer";
  const project = view?.projects.find((candidate) => candidate.id === projectId);
  const online = view?.link.kind === "online";
  const mayStart = online && !!view?.mayStart;
  // That computer reads a name, a model, an effort and a branch, and can clone.
  const manages = mayStart && !!view?.mayManage;
  const targetId = project?.id ?? (cloned && cloned.id === projectId ? cloned.id : null);

  useEffect(() => {
    void useOtherComputersStore.getState().loadHarnesses(hostId);
    useOtherComputersStore.getState().couldNotStart(null);
  }, [hostId]);
  useEffect(() => messageRef.current?.focus(), [hostId, projectId]);

  useEffect(() => {
    if (!from) return;
    let stale = false;
    ipc.projectRepository(from).then(
      (read) => !stale && setRepository(read),
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [from]);

  // Why the project in the list is the one offered, for the project here it was opened in.
  const offered = local
    ? offeredThere(view, ui, hostId, { ...local, repository: repository?.key })
    : null;
  // The same repository is surer than the same name: once this computer has read which one its
  // project is, that one is offered instead, unless the person has chosen already.
  const better =
    !picked && offered?.how === "same repository" && offered.projectId !== projectId
      ? offered.projectId
      : null;
  useEffect(() => {
    if (better) {
      const projects = useProjectsStore.getState();
      projects.composeThere({ ...projects.composingThere, hostId, projectId: better, from });
    }
  }, [better, hostId, from]);

  // That project's branches there, for a computer that takes one.
  const branchesOf = manages ? (project?.id ?? null) : null;
  useEffect(() => {
    if (!branchesOf) return;
    let stale = false;
    ipc.otherHostBranches(hostId, branchesOf).then(
      (list) => {
        if (stale) return;
        setBranches({ of: branchesOf, list });
        const first = list.default ?? list.branches[0];
        setBase(first ? `new:${first}` : "");
      },
      () => {},
    );
    return () => {
      stale = true;
    };
  }, [hostId, branchesOf]);
  const listed = branches && branches.of === project?.id ? branches.list : null;

  const chosen = agent ?? agents?.[0]?.id ?? null;
  const harness = agents?.find((a) => a.id === chosen);
  const effortChoice = harness?.efforts.includes(effort) ? effort : "";
  const [mode, branch] = [base.slice(0, base.indexOf(":")), base.slice(base.indexOf(":") + 1)];
  const openable = listed?.branches.filter((b) => !listed.checkedOut.includes(b)) ?? [];
  // That computer can clone this computer's project when it has none for it.
  const mayClone = manages && !!local && !!repository?.clonable && !project && !cloned;
  const cloning = mayClone && cloneThere;
  const ready =
    busy === null &&
    mayStart &&
    chosen !== null &&
    message.trim() !== "" &&
    (targetId !== null || cloning);

  const why = !local
    ? null
    : cloning
      ? `cloned into ${name}'s projects folder when you start`
      : !project
        ? null
        : offered?.projectId !== project.id
          ? "remembered after you start"
          : offered.how === "remembered"
            ? "as you chose last time"
            : `${offered.how} as here · remembered after you start`;

  // What This computer goes back to: the project it was opened in, or one here of the same name.
  const here =
    local ??
    localProjects.find((candidate) => candidate.name === project?.name) ??
    localProjects.find((candidate) => candidate.name.toLowerCase() === project?.name.toLowerCase());

  const start = async () => {
    if (!ready || starting.current) return;
    starting.current = true;
    const others = useOtherComputersStore.getState();
    let target = targetId;
    if (target === null && from) {
      // That computer clones it with its own Git sign-in, into its own projects folder. Only the
      // repository's address goes over, read by the core from this project.
      setBusy("cloning");
      others.couldNotStart(null);
      try {
        const made = await ipc.otherHostClone(hostId, from);
        target = made.projectId;
        // Cloned, it is a project there whatever happens next: a start that fails after this
        // is tried again in it, and never clones a second time.
        setCloned({ id: made.projectId, name: made.name });
        setCloneThere(false);
        useProjectsStore
          .getState()
          .composeThere({ hostId, projectId: made.projectId, from, message });
      } catch (error) {
        others.couldNotStart(errorMessage(error));
        starting.current = false;
        setBusy(null);
        return;
      }
    }
    if (target === null) {
      starting.current = false;
      return;
    }
    setBusy("starting");
    const projectThere = target;
    const said = (text: string) => (manages && text.trim() !== "" ? text.trim() : null);
    const onBranch = manages && listed !== null && branch !== "";
    const started = await startThere(hostId, () =>
      ipc.otherHostStartWorkspace(hostId, {
        projectId: projectThere,
        prompt: message,
        harness: chosen,
        name: mode === "open" ? null : said(workspaceName),
        model: said(model),
        effort: said(effortChoice),
        baseBranch: onBranch && mode === "new" ? branch : null,
        existingBranch: onBranch && mode === "open" ? branch : null,
      }),
    );
    if (started && from) {
      const projects = useProjectsStore.getState();
      projects.remember(PROJECT_THERE, {
        ...recall<Record<string, string>>(projects.ui, PROJECT_THERE, {}),
        [thereKey(hostId, from)]: projectThere,
      });
    }
    // Started, it has left for the new workspace; not started, the message is still here.
    starting.current = false;
    setBusy(null);
  };

  const chooseProject = (id: string) => {
    setPicked(true);
    setCloneThere(id === CLONE);
    if (id !== CLONE) {
      useProjectsStore.getState().composeThere({ hostId, projectId: id || null, from, message });
    }
  };

  /** Another computer, or this one: what was typed goes along, and so does the project. */
  const runOn = (target: string | null) => {
    const projects = useProjectsStore.getState();
    if (target === null) {
      if (here) projects.compose(here.id, undefined, message || undefined);
      return;
    }
    const like = local
      ? { ...local, repository: repository?.key }
      : project
        ? { id: null, name: project.name, repository: project.repository }
        : null;
    const there = like
      ? offeredThere(useOtherComputersStore.getState().views[target], projects.ui, target, like)
      : null;
    projects.composeThere({ hostId: target, projectId: there?.projectId ?? null, from, message });
  };

  const projectChoices = [
    ...(view?.projects ?? []),
    // Cloned a moment ago, and not yet in what that computer last showed.
    ...(cloned && !view?.projects.some((p) => p.id === cloned.id) ? [cloned] : []),
  ];

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto p-6">
      <form
        aria-label={`New workspace on ${name}`}
        className="w-full max-w-2xl"
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <div className="mb-3 text-center text-ink-muted">
          <h1 className="inline">
            New workspace in{" "}
            <span className="font-medium text-ink">
              {local?.name ?? project?.name ?? "a project"}
            </span>{" "}
            on
          </h1>{" "}
          <RunOn
            hostId={hostId}
            here={here ? null : `no project called ${project?.name ?? local?.name ?? "that"} here`}
            onChoose={runOn}
            disabled={busy !== null}
          />
        </div>
        {view && (projectChoices.length > 0 || mayClone) && (
          <div className="mb-2 flex flex-wrap items-center justify-center gap-2 text-detail text-ink-faint">
            <label htmlFor={projectFieldId}>Project on {name}</label>
            <select
              id={projectFieldId}
              value={cloning ? CLONE : (targetId ?? "")}
              disabled={busy !== null}
              onChange={(event) => chooseProject(event.target.value)}
              className="h-7 max-w-56 rounded border border-line bg-canvas px-2 text-ink outline-none focus:border-accent disabled:opacity-50"
            >
              {targetId === null && !cloning && <option value="">choose one…</option>}
              {projectChoices.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.name}
                </option>
              ))}
              {mayClone && <option value={CLONE}>clone {local?.name} there</option>}
            </select>
            {why && <span>{why}</span>}
          </div>
        )}
        <textarea
          ref={messageRef}
          aria-label="What should the agent work on?"
          placeholder="What should the agent work on?"
          value={message}
          rows={5}
          disabled={busy !== null}
          onChange={(event) => setMessage(event.target.value)}
          onKeyDown={(event) => {
            // Enter sends, Shift+Enter breaks the line — and never while an IME is composing.
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void start();
            } else if (event.key === "Escape") {
              useProjectsStore.getState().composeThere(null);
            }
          }}
          className="w-full resize-none rounded-lg border border-line bg-surface p-3 text-[15px] leading-relaxed outline-none select-text focus:border-accent disabled:opacity-60"
        />

        {manages && mode !== "open" && (
          <div className="mt-2 flex items-center gap-2 text-detail text-ink-faint">
            <label htmlFor={nameFieldId} className="shrink-0">
              Name
            </label>
            <input
              id={nameFieldId}
              value={workspaceName}
              placeholder={`named by ${name} after your message`}
              maxLength={80}
              disabled={busy !== null}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => setWorkspaceName(event.target.value)}
              title="The new workspace's folder there, and its branch after that computer's prefix"
              className="h-7 min-w-0 flex-1 rounded border border-line bg-canvas px-2 font-mono text-ink outline-none select-text focus:border-accent disabled:opacity-50"
            />
          </div>
        )}

        <div className="mt-2 flex flex-wrap items-center gap-2">
          <span className="relative flex items-center">
            {chosen && (
              <HarnessIcon
                id={chosen}
                label={harness?.name}
                className="pointer-events-none absolute left-2"
              />
            )}
            <select
              aria-label="Harness"
              value={chosen ?? ""}
              disabled={busy !== null || !agents || agents.length === 0}
              onChange={(event) => {
                // A model name means nothing to a different agent.
                if (event.target.value !== chosen) setModel("");
                setAgent(event.target.value);
              }}
              className={`${control} ${chosen ? "pl-7" : ""}`}
            >
              {(agents ?? []).map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </select>
          </span>

          {manages && (
            <>
              <input
                aria-label="Model"
                placeholder="model: default"
                list={modelsId}
                value={model}
                maxLength={64}
                disabled={busy !== null}
                spellCheck={false}
                autoComplete="off"
                onChange={(event) => setModel(event.target.value)}
                className={`${control} w-40 select-text`}
              />
              <datalist id={modelsId}>
                {harness?.models.map((suggested) => (
                  <option key={suggested} value={suggested} />
                ))}
              </datalist>
              {harness && harness.efforts.length > 0 && (
                <select
                  aria-label="Effort"
                  value={effortChoice}
                  disabled={busy !== null}
                  onChange={(event) => setEffort(event.target.value)}
                  className={control}
                >
                  <option value="">effort: default</option>
                  {harness.efforts.map((level) => (
                    <option key={level} value={level}>
                      effort: {level}
                    </option>
                  ))}
                </select>
              )}
              {listed && (
                <select
                  aria-label="Branch"
                  title={`Start a new branch from one of ${name}'s, or open one of its branches as it is`}
                  value={base}
                  disabled={busy !== null}
                  onChange={(event) => setBase(event.target.value)}
                  className={`${control} ml-auto max-w-56 font-mono text-detail`}
                >
                  <optgroup label="New branch from">
                    {listed.branches.map((b) => (
                      <option key={b} value={`new:${b}`}>
                        {b}
                      </option>
                    ))}
                  </optgroup>
                  {openable.length > 0 && (
                    <optgroup label="Open existing branch">
                      {openable.map((b) => (
                        <option key={b} value={`open:${b}`}>
                          {b}
                        </option>
                      ))}
                    </optgroup>
                  )}
                </select>
              )}
            </>
          )}
          <button
            type="submit"
            disabled={!ready}
            className={`h-9 rounded bg-accent px-4 font-medium text-canvas disabled:opacity-40 ${
              manages && listed ? "" : "ml-auto"
            }`}
          >
            {busy === "cloning" ? "Cloning…" : busy ? "Starting…" : "Start"}
          </button>
        </div>

        <p className="mt-2 text-detail text-ink-faint">
          {manages ? (
            <>
              The agents, models and branches listed are {name}&rsquo;s own
              {listed?.more ? ", and it has more branches than are listed" : ""}. {name} adds its
              project memory and runs its setup, as its settings say. Triggers for the new workspace
              are set there.
            </>
          ) : (
            <>
              The agents listed are {name}&rsquo;s own. It names the branch after your message,
              starts from its default branch, adds its project memory and runs its setup, as its
              settings say.
              {mayStart &&
                " Its Yardsort is older than this one, so a name, a model, an effort and a branch cannot be chosen from here."}
            </>
          )}
        </p>

        <div className="mt-3 min-h-10 text-center">
          {problem ? (
            <p role="alert" className="text-red-400 select-text">
              {problem}
            </p>
          ) : view && !online ? (
            <p role="alert" className="text-amber-300">
              {name} is {linkText(view.link)}. Nothing can be started there until it is back, and
              nothing is queued for it.
            </p>
          ) : view && !view.mayStart ? (
            <p role="alert" className="text-amber-300">
              {name} lets this computer watch, not start work.
            </p>
          ) : busy === "cloning" ? (
            <p className="text-ink-faint">
              {name} is cloning {local?.name} into its projects folder, with its own Git sign-in. A
              large repository takes a while.
            </p>
          ) : view && targetId === null && !cloning ? (
            <p role="alert" className="text-amber-300">
              {local
                ? `${name} has no project called ${local.name}.`
                : `${name} no longer shows that project.`}{" "}
              {mayClone ? (
                <>
                  <button
                    type="button"
                    onClick={() => chooseProject(CLONE)}
                    className="underline decoration-line underline-offset-2 hover:text-ink"
                  >
                    Clone it there
                  </button>
                  : {name} clones the repository into its own projects folder, with its own Git
                  sign-in, and the workspace is made in it. Or add it there yourself
                </>
              ) : (
                <>
                  Add it there, with Add project in its window or{" "}
                  <span className="font-mono">ys project add</span> in its terminal, and it is
                  offered here
                </>
              )}
              .{projectChoices.length > 0 && " Or choose another of its projects above."}
            </p>
          ) : agents?.length === 0 ? (
            <p role="alert" className="text-red-400">
              {name} has no agent it can start. Install one there first.
            </p>
          ) : busy ? (
            <p className="text-ink-faint">
              Making the workspace and starting the agent on {name}. Its setup can take a while.
            </p>
          ) : (
            <p className="text-ink-faint">
              Enter to start · Shift+Enter for a new line · Esc to cancel.{" "}
              {cloning
                ? `${name} clones ${local?.name} first, then makes a new branch and git worktree in it.`
                : mode === "open"
                  ? `Opens the existing branch "${branch}" in a new git worktree on ${name}.`
                  : `A new branch and git worktree are created on ${name} when you start.`}{" "}
              Nothing is created on this computer.
            </p>
          )}
        </div>
      </form>
    </div>
  );
}
