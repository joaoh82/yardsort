import { useState, type ReactNode } from "react";
import { errorMessage, ipc } from "@/lib/ipc";
import { useProjectsStore } from "@/stores/projects";
import { ShareDialog, type Question } from "./ShareDialog";

/**
 * Ask once per project, before an agent starts beside a conversation that has said something,
 * whether the project's agents may read each other's conversations. `ask` resolves `true` when
 * there is nothing to ask or the user answered, either way — the launch goes ahead and the core
 * gives the note or not — and `false` when they closed the question, which starts nothing.
 */
export function useShareQuestion(): {
  ask: (workspaceId: string) => Promise<boolean>;
  question: ReactNode;
} {
  const [question, setQuestion] = useState<Question | null>(null);

  const ask = async (workspaceId: string) => {
    const project = useProjectsStore
      .getState()
      .projects.find((candidate) => candidate.workspaces.some((w) => w.id === workspaceId));
    const about = project ? await ipc.conversationsUnasked(workspaceId).catch(() => null) : null;
    if (!project || !about) return true;
    return new Promise<boolean>((resolve) =>
      setQuestion({ about, projectId: project.id, projectName: project.name, answer: resolve }),
    );
  };

  const close = (go: boolean) => {
    question?.answer(go);
    setQuestion(null);
  };

  return {
    ask,
    question: question && (
      <ShareDialog
        question={question}
        onAnswer={async (share) => {
          try {
            await ipc.conversationsShare(question.projectId, share);
            close(true);
          } catch (error) {
            useProjectsStore.setState({ error: errorMessage(error) });
            close(false);
          }
        }}
        onClose={() => close(false)}
      />
    ),
  };
}
