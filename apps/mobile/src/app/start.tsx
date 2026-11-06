// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Starting work from the phone: a new workspace with an agent and its first message, or another
 * agent in a workspace. The computer does it as its own window would, with its own settings: the
 * phone names only the project or workspace, one of the agents the computer offers, and the message.
 * Once it has started, its terminal opens.
 *
 * Nothing is started twice. A start whose answer is lost on the way is said to be so, and the
 * person looks at the computer's list before starting again.
 */

import {
  ChannelClosed,
  type Channel,
  type Cursor,
  type HarnessSummary,
  type HostId,
  type ProjectId,
  type WorkspaceId,
  type WorkspacePage,
} from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useState } from "react";

import { messageOf } from "../lib/accounts";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Button, Field, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";

/** How many pages of workspaces are read for the projects: thousands of workspaces. */
const MOST_PAGES = 40;

/** What a computer answers that can start work: one from before cannot. */
export function canStart(channel: Channel | null): boolean {
  return channel?.host.capabilities.includes("workspace.start") ?? false;
}

export default function StartScreen() {
  const params = useLocalSearchParams<{ host?: string; workspace?: string; project?: string }>();
  if (!params.host) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Start
        host={params.host as HostId}
        workspace={(params.workspace as WorkspaceId | undefined) ?? null}
        project={(params.project as ProjectId | undefined) ?? null}
      />
    </OfProfile>
  );
}

/** What there is to choose from, read from the computer. */
interface Choices {
  harnesses: HarnessSummary[];
  /** The projects a new workspace can be made in. Empty for another agent in a workspace. */
  projects: { id: ProjectId; name: string }[];
  /** The workspace another agent joins, by name. */
  workspaceName: string | null;
}

function Start({
  host,
  workspace,
  project: wantedProject,
}: {
  host: HostId;
  workspace: WorkspaceId | null;
  project: ProjectId | null;
}) {
  const router = useRouter();
  const { reach, channel } = useChannelTo(host);
  const [asked, setAsked] = useState(0);
  const [project, setProject] = useState<ProjectId | null>(wantedProject);
  const [harness, setHarness] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [starting, setStarting] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const read = useCallback(
    async (through: Channel): Promise<Choices> => {
      if (!canStart(through)) return { harnesses: [], projects: [], workspaceName: null };
      const harnesses = await through.harnesses();
      if (workspace) {
        const detail = await through.workspace(workspace);
        return { harnesses, projects: [], workspaceName: detail.workspace.name };
      }
      // A page names only the projects of its workspaces, and every project has one, its local
      // workspace: so every page is read, for a project whose workspaces are all on a later one.
      const projects = new Map<ProjectId, string>();
      let cursor: Cursor | null = null;
      for (let pages = 0; pages < MOST_PAGES; pages += 1) {
        const page: WorkspacePage = await through.workspaces({ cursor });
        for (const p of page.projects) if (!p.missing) projects.set(p.id, p.name);
        cursor = page.next;
        if (!cursor) break;
      }
      return {
        harnesses,
        projects: [...projects].map(([id, name]) => ({ id, name })),
        workspaceName: null,
      };
    },
    [workspace],
  );
  const choices = useRead(channel, read, asked);

  const title = workspace ? "New agent" : "New workspace";
  const screenTitle = <Stack.Screen options={{ title }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {screenTitle}
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  const answer = choices.answer;
  const agents = answer?.harnesses ?? [];
  const chosenAgent = harness ?? agents[0]?.id ?? null;
  const projects = answer?.projects ?? [];
  const chosenProject = project ?? (projects.length === 1 ? (projects[0]?.id ?? null) : null);
  const ready =
    channel !== null &&
    chosenAgent !== null &&
    !starting &&
    (workspace ? true : chosenProject !== null && message.trim() !== "");

  const start = async () => {
    if (!channel || !chosenAgent) return;
    setStarting(true);
    setProblem(null);
    try {
      const started = workspace
        ? await channel.startSession({
            workspace,
            harness: chosenAgent,
            ...(message.trim() ? { prompt: message } : {}),
          })
        : await channel.startWorkspace({
            project: chosenProject as ProjectId,
            harness: chosenAgent,
            prompt: message,
          });
      const name = agents.find((a) => a.id === started.harness)?.name ?? started.harness;
      router.replace({
        pathname: "/terminal",
        params: { host, terminal: started.terminal, label: name, workspace: started.workspace },
      });
    } catch (error) {
      setStarting(false);
      setProblem(
        error instanceof ChannelClosed
          ? "The computer's answer did not arrive, so it may have started anyway. Look at its list before starting again."
          : messageOf(error),
      );
    }
  };

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      {screenTitle}
      <ReachLine reach={reach} lastSeenAt={null} />
      {choices.problem && <Problem>{choices.problem}</Problem>}
      {channel && !answer && !choices.problem && <Waiting label="Asking the computer" />}
      {!channel && <Body muted>Starting needs the computer to be reachable.</Body>}
      {channel && answer && !canStart(channel) && (
        <Body muted>
          This computer’s Yardsort is too old to start work from a phone. Update it, then try again.
        </Body>
      )}
      {answer && canStart(channel) && agents.length === 0 && (
        <Body muted>The computer has no agent it can start. Install one there first.</Body>
      )}

      {answer && workspace && answer.workspaceName && (
        <Body>
          Another agent in {answer.workspaceName}, with the same files as the others there.
        </Body>
      )}

      {answer && !workspace && projects.length > 0 && (
        <Section title="Project">
          <Group>
            {projects.map((p) => (
              <Row
                key={p.id}
                title={p.name}
                checked={p.id === chosenProject}
                onPress={() => setProject(p.id)}
              />
            ))}
          </Group>
        </Section>
      )}

      {agents.length > 0 && (
        <Section title="Agent">
          <Group>
            {agents.map((a) => (
              <Row
                key={a.id}
                title={a.name}
                checked={a.id === chosenAgent}
                onPress={() => setHarness(a.id)}
              />
            ))}
          </Group>
        </Section>
      )}

      {agents.length > 0 && (
        <Field
          label={workspace ? "First message, if any" : "What should it do?"}
          hint={
            workspace
              ? "Without one, the agent waits for you."
              : "The workspace’s branch is named after it."
          }
          value={message}
          onChangeText={setMessage}
          multiline
          autoCapitalize="sentences"
          editable={!starting}
        />
      )}

      {problem && <Problem>{problem}</Problem>}
      {starting && !workspace && (
        <Body muted>Making the workspace and starting the agent. Its setup can take a while.</Body>
      )}
      {agents.length > 0 && (
        <Button
          title={workspace ? "Start agent" : "Start workspace"}
          kind="primary"
          busy={starting}
          disabled={!ready}
          onPress={() => void start()}
        />
      )}
    </Screen>
  );
}
