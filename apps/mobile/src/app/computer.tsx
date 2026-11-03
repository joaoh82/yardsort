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

import { messageOf } from "../lib/accounts";
import { useApp } from "../lib/context";
import { moment } from "../lib/format";
import type { ObservedComputer, ObservedWorkspaces } from "../lib/observed";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { Body, Button, Label, Problem, Row, Screen, Title, Waiting } from "../ui/parts";

/** A workspace as this screen shows it, from the computer or from what was last seen. */
interface Shown {
  id: WorkspaceId;
  project: ProjectId;
  name: string;
  detail: string;
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
      <Title>Workspaces</Title>
      {first.problem && <Problem>{first.problem}</Problem>}
      {problem && <Problem>{problem}</Problem>}
      {oldShown && <Body muted>As of {moment(seen.observedAt)}.</Body>}
      {page && !channel && first.at !== null && <Body muted>As of {moment(first.at)}.</Body>}
      {!shown && !first.problem && (
        <Waiting label={channel ? "Asking the computer" : "Waiting for the computer"} />
      )}
      {shown?.length === 0 && <Body muted>Nothing here this phone may see.</Body>}
      {projects.map((project) => {
        const here = shown?.filter((workspace) => workspace.project === project.id) ?? [];
        if (here.length === 0) return null;
        return (
          <Section key={project.id} name={project.name}>
            {here.map((workspace) => (
              <Row
                key={workspace.id}
                title={workspace.name}
                {...(workspace.detail ? { detail: workspace.detail } : {})}
                onPress={() =>
                  router.push({
                    pathname: "/workspace",
                    params: { host, workspace: workspace.id },
                  })
                }
              />
            ))}
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
    </Screen>
  );
}

function Section({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <>
      <Label>{name}</Label>
      {children}
    </>
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
    detail: [
      workspace.missing ? "Its folder is gone" : on,
      running > 0 ? `${running} running` : null,
    ]
      .filter(Boolean)
      .join(" · "),
  };
}
