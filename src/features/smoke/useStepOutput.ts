import { useEffect, useRef, useState } from "react";
import { hasCore, ipc, type OutputChunk } from "@/lib/ipc";

/** More text than this is dropped from the front, so a chatty server cannot grow the page. */
const KEEP_CHARS = 1_000_000;

export interface StepOutput {
  text: string;
  /** Lines before `text` were not read, or were dropped: the file is bigger than what shows. */
  skipped: boolean;
  /** The first read has come back. */
  loaded: boolean;
}

export interface OutputSource {
  /** A piece of the file from `from`. `ipc.testStepOutput` unless a test says otherwise. */
  read: (
    runId: string,
    stepId: string,
    stream: "stdout" | "stderr",
    from: number,
  ) => Promise<OutputChunk>;
  /** Called whenever a step's file grew. `ipc.onTestRunOutput` unless a test says otherwise. */
  subscribe: (handler: (runId: string, stepId: string) => void) => Promise<() => void>;
}

const core: OutputSource = {
  read: (runId, stepId, stream, from) => ipc.testStepOutput(runId, stepId, stream, from),
  subscribe: (handler) => ipc.onTestRunOutput(handler),
};

/**
 * A step's stdout or stderr, tailed: read once, then read on from where the last read ended
 * whenever the runner says the file grew, and once more when the step ends, for the last lines
 * written after the last notice.
 */
export function useStepOutput(
  runId: string,
  stepId: string,
  stream: "stdout" | "stderr",
  live: boolean,
  source: OutputSource = core,
): StepOutput {
  const [output, setOutput] = useState<StepOutput>({ text: "", skipped: false, loaded: false });
  const next = useRef(0);
  // The current effect's serialized `read`, so the end-of-step read goes through the same
  // one-at-a-time gate and never races a read still in flight.
  const readNow = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => {
    if (!hasCore() && source === core) return;
    let current = true;
    next.current = 0;
    // What is in flight belongs to this run of the effect alone: React mounts twice in
    // development, and a read the first mount started must not stand in for the second's.
    let reading: Promise<void> | null = null;
    let again = false;

    // One read at a time: a notice that lands mid-read asks for one more read afterwards,
    // from where that one ended, so nothing is read twice or out of order.
    const read = (): Promise<void> => {
      if (reading) {
        again = true;
        return reading;
      }
      const from = next.current;
      reading = source
        .read(runId, stepId, stream, from)
        .then((chunk) => {
          if (!current) return;
          next.current = chunk.next;
          // The first read of a step starts the text over; the later ones add to it.
          setOutput((shown) => {
            let text = from === 0 ? chunk.text : shown.text + chunk.text;
            let skipped = (from !== 0 && shown.skipped) || chunk.from > from;
            if (text.length > KEEP_CHARS) {
              text = text.slice(text.length - KEEP_CHARS);
              skipped = true;
            }
            return { text, skipped, loaded: true };
          });
        })
        .catch(() => {
          if (current) setOutput((shown) => ({ ...shown, loaded: true }));
        })
        .finally(() => {
          reading = null;
          if (again && current) {
            again = false;
            void read();
          }
        });
      return reading;
    };

    readNow.current = read;
    void read();
    const unlisten = source.subscribe((grewRun, grewStep) => {
      if (grewRun === runId && grewStep === stepId) void read();
    });
    return () => {
      current = false;
      readNow.current = null;
      void unlisten.then((stop) => stop());
    };
  }, [runId, stepId, stream, source]);

  // The step ended: whatever was written after the last notice is read now, through the same
  // serialized `read`, so it queues behind any read still in flight rather than reading twice.
  const wasLive = useRef(live);
  useEffect(() => {
    if (wasLive.current && !live) void readNow.current?.();
    wasLive.current = live;
  }, [live, runId, stepId, stream, source]);

  return output;
}
