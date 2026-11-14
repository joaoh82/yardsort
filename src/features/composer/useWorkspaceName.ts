import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, type SuggestedName } from "@/lib/ipc";

/** How long typing must pause before the message's own words are turned into a name. */
const WORDS_DELAY_MS = 150;
/**
 * How long typing must pause before a model is asked. Longer than Assist's pause: asking may
 * start an agent, and someone still typing will only change the message again.
 */
export const NAME_DELAY_MS = 1_500;
/** Shorter than this, the message's own words are all the name there is to find. */
export const NAME_FROM_CHARS = 15;

export interface WorkspaceNaming {
  /** What the Name field shows: empty when there is nothing to show yet. */
  shown: string;
  /** What would be used if the field were left alone: the suggestion behind a name you typed. */
  suggested: string;
  /** Who chose what is shown: a model's or agent's name, or `null` for the message's words. */
  by: string | null;
  /** The user has typed in the field, so no suggestion replaces what is there. */
  edited: boolean;
  /** A model is being asked right now. */
  naming: boolean;
  /** Why the last model asked gave no name, when it did not. */
  problem: string | null;
  edit: (value: string) => void;
  /** Forget the name you typed and go back to the suggestion. */
  reset: () => void;
  /**
   * The name to start with — waiting for the model first when one should be asked and has not
   * answered for this message. `undefined` leaves naming to the core, which uses the message's
   * words exactly as the field showed them.
   */
  settle: () => Promise<string | undefined>;
  /** Stop waiting in `settle` and go ahead with what is shown. */
  skip: () => void;
}

/**
 * The name a new workspace will get, shown before it is made: a folder and a branch cannot be
 * renamed under a running agent, so this is the one chance to get it right.
 *
 * The message's own words fill the field as you type. With writing switched on and somebody
 * able to write (`withModel`), a model is asked once typing pauses — never more than one request
 * at a time — and its answer replaces the words. A name you type is yours: nothing replaces it
 * until you reset it. Emptied, the field shows the suggestion faintly, and that is what is used.
 */
export function useWorkspaceName(
  projectId: string,
  message: string,
  harnessId: string | null,
  enabled: boolean,
  withModel: boolean,
): WorkspaceNaming {
  const text = message.trim();
  const [words, setWords] = useState<{ forText: string; name: string | null } | null>(null);
  const [answer, setAnswer] = useState<(SuggestedName & { forText: string }) | null>(null);
  const [own, setOwn] = useState<string | null>(null);
  const [naming, setNaming] = useState(false);
  const inflight = useRef<{ text: string; promise: Promise<SuggestedName | null> } | null>(null);
  const skipping = useRef<(() => void) | null>(null);
  const asking = enabled && withModel;

  // The message's words, from the core so they are exactly what it would name it.
  useEffect(() => {
    if (!enabled || !text) return;
    let stale = false;
    const timer = setTimeout(() => {
      ipc.workspaceSuggestName(projectId, text, false, null).then(
        (found) => !stale && setWords({ forText: text, name: found.name }),
        () => {},
      );
    }, WORDS_DELAY_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [enabled, projectId, text]);

  // The message most recently asked about: only its answer is kept, so a slower answer about an
  // older message can never replace a newer one.
  const latest = useRef<string | null>(null);
  const ask = useCallback(
    (wanted: string): Promise<SuggestedName | null> => {
      latest.current = wanted;
      const running = inflight.current;
      if (running?.text === wanted) return running.promise;
      setNaming(true);
      // One request at a time, even when Start asks about a message edited since the last one:
      // wait for that one to finish (its answer is dropped as stale), then ask.
      const promise = (running ? running.promise.then(() => undefined) : Promise.resolve())
        .then(() => ipc.workspaceSuggestName(projectId, wanted, true, harnessId))
        .then(
          (found) => {
            if (latest.current === wanted) setAnswer({ ...found, forText: wanted });
            return found;
          },
          () => null,
        );
      inflight.current = { text: wanted, promise };
      void promise.finally(() => {
        if (inflight.current?.promise === promise) inflight.current = null;
        setNaming(inflight.current !== null);
      });
      return promise;
    },
    [projectId, harnessId],
  );

  // Ask the model once typing pauses — and again after an answer, if the message moved on
  // meanwhile. One request at a time: `naming` re-runs this when the last one is done.
  useEffect(() => {
    if (!asking || own !== null || naming || text.length < NAME_FROM_CHARS) return;
    if (answer?.forText === text) return;
    const timer = setTimeout(() => void ask(text), NAME_DELAY_MS);
    return () => clearTimeout(timer);
  }, [asking, own, naming, text, answer, ask]);

  // Only an answer about the message as it stands: an older one could name a workspace Start
  // would not make — a message cut below the model's minimum is never asked about again.
  const modelName = answer?.forText === text && answer.name && answer.by ? answer : null;
  const suggested = !text
    ? ""
    : (modelName?.name ?? (words?.forText === text ? (words.name ?? "") : ""));

  const settle = async (): Promise<string | undefined> => {
    if (!enabled) return undefined;
    // An emptied field shows the suggestion faintly, and means it.
    if (own !== null && own.trim()) return own.trim();
    if (!asking || text.length < NAME_FROM_CHARS) return undefined;
    if (answer?.forText === text) return (answer.by && answer.name) || undefined;
    const skipped = new Promise<null>((resolve) => (skipping.current = () => resolve(null)));
    const found = await Promise.race([ask(text), skipped]);
    skipping.current = null;
    return (found?.by && found.name) || undefined;
  };

  return {
    shown: own ?? suggested,
    suggested,
    by: own !== null ? null : (modelName?.by ?? null),
    edited: own !== null,
    naming,
    problem: own === null && answer?.forText === text ? answer.problem : null,
    edit: setOwn,
    reset: () => setOwn(null),
    settle,
    skip: () => skipping.current?.(),
  };
}
