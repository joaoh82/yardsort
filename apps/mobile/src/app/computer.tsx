// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One computer: its workspaces, by project. What was last seen shows, marked as such, until the
 * computer answers, or for as long as it cannot.
 */

import type {
  Channel,
  HostId,
  ProjectId,
  WorkspaceId,
  WorkspacePage,
  WorkspaceSummary,
} from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useState } from "react";
import { Alert } from "react-native";

import { messageOf } from "../lib/accounts";
import { useApp } from "../lib/context";
import { moment } from "../lib/format";
import type { ObservedComputer, ObservedWorkspaces } from "../lib/observed";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { canStart } from "./start";
import { Body, Button, Group, Problem, Row, Screen, Section, Waiting } from "../ui/parts";

/** A workspace as this screen shows it, from the computer or from what was last seen. */
interface Shown {
  id: WorkspaceId;
  project: ProjectId;
  name: string;
  detail: string;
  running?: string | undefined;
}

export default function ComputerScreen() {
  const params = useLocalSearchParams<{ host?: string }>();
  if (!params.host) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Computer host={params.host as HostId} />
    </OfProfile>
  );
}

function Computer({ host }: { host: HostId }) {
  const { accounts } = useApp();
  const router = useRouter();
  const { reach, channel } = useChannelTo(host);
  const [computer, setComputer] = useState<ObservedComputer | null>(null);
  const [seen, setSeen] = useState<ObservedWorkspaces | null>(null);
  const [asked, setAsked] = useState(0);
  const [more, setMore] = useState<WorkspacePage | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [unpairing, setUnpairing] = useState(false);

  const unpair = async () => {
    setUnpairing(true);
    setProblem(null);
    try {
      // Which grant is the server's to say, so it is asked only when needed.
      const paired = (await accounts.computers()).computers.find((c) => c.host === host);
      if (paired) await accounts.unpair(paired.grant, host);
      router.dismissTo("/account");
    } catch (error) {
      setProblem(messageOf(error));
      setUnpairing(false);
    }
  };

  useEffect(() => {
    const observed = accounts.observed();
    if (!observed) return;
    let stale = false;
    void Promise.all([observed.computers(), observed.workspaces(host)]).then(
      ([computers, workspaces]) => {
        if (stale) return;
        setComputer(computers.find((c) => c.host === host) ?? null);
        setSeen(workspaces);
      },
    );
    return () => {
      stale = true;
    };
  }, [accounts, host]);

  const read = useCallback(
    async (through: Channel) => {
      const page = await through.workspaces();
      setMore(null);
      await accounts.observed()?.keepWorkspaces(host, page);
      return page;
    },
    [accounts, host],
  );
  const first = useRead(channel, read, asked);

  const showMore = async (cursor: NonNullable<WorkspacePage["next"]>) => {
    if (!channel) return;
    try {
      const page = await channel.workspaces({ cursor });
      const held = more ?? first.answer;
      if (!held) return;
      setMore({
        projects: [
          ...held.projects,
          ...page.projects.filter((p) => !held.projects.some((h) => h.id === p.id)),
        ],
        workspaces: [...held.workspaces, ...page.workspaces],
        next: page.next,
      });
    } catch (error) {
      setProblem(messageOf(error));
    }
  };

  const name = channel?.host.name ?? computer?.name ?? "Computer";
  const title = <Stack.Screen options={{ title: name }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {title}
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  const page = more ?? first.answer;
  const shown: Shown[] | null = page
    ? page.workspaces.map(shownFromComputer)
    : seen
      ? seen.workspaces.map((workspace) => ({
          id: workspace.id,
          project: workspace.project,
          name: workspace.name,
          detail: workspace.missing ? "Its folder is gone" : (workspace.branch ?? ""),
        }))
      : null;
  const projects = page?.projects ?? seen?.projects ?? [];
  const oldShown = !page && seen;

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      {title}
      <ReachLine reach={reach} lastSeenAt={computer?.lastSeenAt ?? null} />
      {first.problem && <Problem>{first.problem}</Problem>}
      {problem && <Problem>{problem}</Problem>}
      {oldShown && <Body muted>As of {moment(seen.observedAt)}.</Body>}
      {page && !channel && first.at !== null && <Body muted>As of {moment(first.at)}.</Body>}
      {!shown && !first.problem && (
        <Waiting label={channel ? "Asking the computer" : "Waiting for the computer"} />
      )}
      {shown?.length === 0 && <Body muted>Nothing here this phone may see.</Body>}
      {canStart(channel) && (
        <Button
          title="New workspace"
          kind="primary"
          onPress={() => router.push({ pathname: "/start", params: { host } })}
        />
      )}
      <Button
        title="Workflows"
        onPress={() => router.push({ pathname: "/workflows", params: { host } })}
      />
      {projects.map((project) => {
        const here = shown?.filter((workspace) => workspace.project === project.id) ?? [];
        if (here.length === 0) return null;
        return (
          <Section key={project.id} title={project.name} note={here.length}>
            <Group>
              {here.map((workspace) => (
                <Row
                  key={workspace.id}
                  title={workspace.name}
                  {...(workspace.detail ? { detail: workspace.detail } : {})}
                  monoDetail
                  {...(workspace.running ? { badge: workspace.running } : {})}
                  label={[
                    workspace.name,
                    [workspace.detail, workspace.running].filter(Boolean).join(" · "),
                  ]
                    .filter(Boolean)
                    .join(", ")}
                  onPress={() =>
                    router.push({
                      pathname: "/workspace",
                      params: { host, workspace: workspace.id },
                    })
                  }
                />
              ))}
            </Group>
          </Section>
        );
      })}
      {page?.next && (
        <Button
          title="Show more"
          onPress={() => {
            if (page.next) void showMore(page.next);
          }}
        />
      )}
      <Button
        title={`Unpair ${name}`}
        kind="danger"
        busy={unpairing}
        onPress={() => confirmUnpair(name, () => void unpair())}
      />
    </Screen>
  );
}

function confirmUnpair(name: string, unpair: () => void): void {
  Alert.alert(
    `Unpair ${name}?`,
    "This phone stops reaching it, and stays signed in. To reach it again, pair again and say yes at that computer.",
    [
      { text: "Cancel", style: "cancel" },
      { text: "Unpair", style: "destructive", onPress: unpair },
    ],
  );
}

function shownFromComputer(workspace: WorkspaceSummary): Shown {
  const running = workspace.runningSessions;
  const branch = workspace.branch;
  const on = !branch
    ? null
    : branch.detached
      ? `detached at ${branch.label}`
      : branch.unborn
        ? `${branch.label}, no commits yet`
        : branch.label;
  return {
    id: workspace.id,
    project: workspace.project,
    name: workspace.name,
    detail: (workspace.missing ? "Its folder is gone" : on) ?? "",
    running: running > 0 ? `${running} running` : undefined,
  };
}
