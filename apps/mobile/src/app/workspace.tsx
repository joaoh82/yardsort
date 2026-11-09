// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One workspace: its conversations and other terminals, and what has been recorded there, followed
 * live. None of it is kept: it shows only when the computer can be reached, and goes with the screen.
 */

import type {
  Channel,
  HostId,
  SessionPage,
  SessionSummary,
  TerminalPurpose,
  TerminalSummary,
  WorkspaceId,
} from "@yardsort/remote-client";
import { Redirect, Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ActivityFeed, describeEvent, reportsOnlyStartAndEnd } from "../lib/activity";
import { messageOf } from "../lib/accounts";
import { useApp } from "../lib/context";
import { moment, stamp } from "../lib/format";
import type { ObservedWorkspace } from "../lib/observed";
import { NoLongerPaired, OfProfile, ReachLine, useChannelTo, useRead } from "../ui/live";
import { canResume, canStart } from "./start";
import {
  Body,
  Button,
  Group,
  FONTS,
  Icon,
  Problem,
  Row,
  Screen,
  Section,
  useColors,
  Waiting,
} from "../ui/parts";

export default function WorkspaceScreen() {
  const params = useLocalSearchParams<{ host?: string; workspace?: string }>();
  if (!params.host || !params.workspace) return <Redirect href="/account" />;
  return (
    <OfProfile>
      <Workspace host={params.host as HostId} workspace={params.workspace as WorkspaceId} />
    </OfProfile>
  );
}

function Workspace({ host, workspace }: { host: HostId; workspace: WorkspaceId }) {
  const { accounts } = useApp();
  const router = useRouter();
  const { reach, channel } = useChannelTo(host);
  const [seen, setSeen] = useState<ObservedWorkspace | null>(null);
  const [asked, setAsked] = useState(0);
  const [ended, setEnded] = useState(false);
  const [more, setMore] = useState<SessionPage | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    void accounts
      .observed()
      ?.workspaces(host)
      .then((kept) => {
        if (!stale) setSeen(kept?.workspaces.find((w) => w.id === workspace) ?? null);
      });
    return () => {
      stale = true;
    };
  }, [accounts, host, workspace]);

  const readDetail = useCallback((through: Channel) => through.workspace(workspace), [workspace]);
  const detail = useRead(channel, readDetail, asked);

  const readSessions = useCallback(
    async (through: Channel) => {
      const page = await through.sessions({ workspace, includeEnded: ended });
      setMore(null);
      return page;
    },
    [workspace, ended],
  );
  const sessions = useRead(channel, readSessions, asked);

  const feed = useMemo(() => new ActivityFeed(workspace), [workspace]);
  const activity = useSyncExternalStore(feed.subscribe, feed.getSnapshot);
  useEffect(() => {
    if (!channel) return;
    void feed.follow(channel);
    return () => feed.stop();
  }, [channel, feed, asked]);

  const showMore = async (cursor: NonNullable<SessionPage["next"]>) => {
    if (!channel) return;
    try {
      const page = await channel.sessions({ workspace, includeEnded: ended, cursor });
      const held = more ?? sessions.answer;
      if (!held) return;
      setMore({
        sessions: [...held.sessions, ...page.sessions],
        terminals: held.terminals,
        next: page.next,
      });
    } catch (error) {
      setProblem(messageOf(error));
    }
  };

  // Not a conversation's title, which is the person's own words and has no place in a route.
  const watch = (terminal: TerminalSummary, label: string) =>
    router.push({
      pathname: "/terminal",
      params: { host, terminal: terminal.id, label, workspace },
    });

  const name = detail.answer?.workspace.name ?? seen?.name ?? "Workspace";
  const project = detail.answer?.project.id ?? seen?.project;
  const title = <Stack.Screen options={{ title: name }} />;

  if (reach?.kind === "gone") {
    return (
      <Screen>
        {title}
        <NoLongerPaired why={reach.why} />
      </Screen>
    );
  }

  const page = more ?? sessions.answer;
  const newestFirst = [...activity.events].reverse();

  return (
    <Screen refreshing={false} onRefresh={() => setAsked((n) => n + 1)}>
      {title}
      <ReachLine reach={reach} lastSeenAt={null} also={detail.answer?.workspace.branch?.label} />
      {!channel && !page && (
        <Body muted>Conversations and activity show when the computer can be reached.</Body>
      )}

      <Section title="Conversations" note={page?.sessions.length}>
        {sessions.problem && <Problem>{sessions.problem}</Problem>}
        {problem && <Problem>{problem}</Problem>}
        {channel && !page && !sessions.problem && <Waiting label="Asking the computer" />}
        {page && !channel && sessions.at !== null && (
          <Body muted>As of {moment(sessions.at)}.</Body>
        )}
        {page?.sessions.length === 0 && (
          <Body muted>{ended ? "No conversations yet." : "No conversations running."}</Body>
        )}
        <Group>
          {page?.sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              resume={
                session.resumable && canResume(channel)
                  ? () =>
                      router.push({
                        pathname: "/start",
                        params: { host, workspace, resume: session.id },
                      })
                  : undefined
              }
              watch={
                session.terminal
                  ? () => watch(session.terminal as TerminalSummary, session.harness)
                  : undefined
              }
            />
          ))}
        </Group>
        {page?.next && (
          <Button
            title="Show more conversations"
            onPress={() => {
              if (page.next) void showMore(page.next);
            }}
          />
        )}
        {canStart(channel) && (
          <Button
            title="New agent"
            onPress={() => router.push({ pathname: "/start", params: { host, workspace } })}
          />
        )}
        {page && (
          <Button
            title={ended ? "Hide ended conversations" : "Show ended conversations"}
            kind="quiet"
            onPress={() => setEnded((was) => !was)}
          />
        )}
      </Section>

      {page && page.terminals.length > 0 && (
        <Section title="Other terminals">
          <Group>
            {page.terminals.map(({ purpose, program, terminal }) => (
              <Row
                key={terminal.id}
                title={purposeWords(purpose, program)}
                plainTitle
                detail={terminalWords(terminal)}
                leading={<Icon name="terminal" />}
                label={`${purposeWords(purpose, program)}, terminal`}
                onPress={() => watch(terminal, purposeWords(purpose, program))}
              />
            ))}
          </Group>
        </Section>
      )}

      {project && (
        <Group>
          <Row
            title="Approved memory"
            plainTitle
            leading={<Icon name="note" />}
            onPress={() => router.push({ pathname: "/memory", params: { host, project } })}
          />
        </Group>
      )}

      <Section title="Activity">
        {activity.problem && <Problem>{activity.problem}</Problem>}
        {channel && !activity.loaded && <Waiting label="Asking the computer" />}
        {activity.following && <Body muted>Following as it is recorded.</Body>}
        {activity.loaded && activity.events.length === 0 && (
          <Body muted>Nothing recorded here yet.</Body>
        )}
        {reportsOnlyStartAndEnd(activity.events) && (
          <Body muted>
            Only starts and ends show for this agent. To see its prompts and tools here, turn on its
            capture at the computer, in Settings, General, Activity, then start a new conversation.
          </Body>
        )}
        <Group>
          {newestFirst.map((event) => {
            const words = describeEvent(event);
            return (
              <EventRow
                key={event.id}
                title={words.title}
                detail={words.detail}
                tone={words.tone}
                at={event.occurredAt}
              />
            );
          })}
        </Group>
        {activity.hasOlder && <Button title="Show older" onPress={() => void feed.older()} />}
      </Section>
    </Screen>
  );
}

/**
 * A conversation. One the computer can resume goes to resuming it, which also shows its last
 * screen while the computer keeps it; any other with a terminal goes to that terminal.
 */
function SessionRow({
  session,
  watch,
  resume,
}: {
  session: SessionSummary;
  watch?: (() => void) | undefined;
  resume?: (() => void) | undefined;
}) {
  const about = [session.harness, session.model].filter(Boolean).join(" · ");
  const open = resume ?? watch;
  return (
    <Row
      title={session.title}
      plainTitle
      about={about}
      detail={sessionWords(session)}
      leading={<AgentMark harness={session.harness} />}
      label={resume ? `${session.title}, ${sessionWords(session)}, resume` : session.title}
      {...(resume ? { tag: "Resume" } : {})}
      {...(open ? { onPress: open } : {})}
    />
  );
}

/** Which agent, as the desktop app colors it. */
function AgentMark({ harness }: { harness: string }) {
  const colors = useColors();
  const color =
    harness === "claude" ? colors.claude : harness === "codex" ? colors.codex : colors.muted;
  return <View style={[styles.mark, { backgroundColor: color }]} />;
}

function EventRow({
  title,
  detail,
  tone,
  at,
}: {
  title: string;
  detail: string;
  tone: "ok" | "bad" | "plain";
  at: number;
}) {
  const colors = useColors();
  const color = tone === "bad" ? colors.danger : tone === "ok" ? colors.online : colors.ink;
  return (
    <View accessible style={styles.event}>
      <Text style={[styles.eventTime, { color: colors.faint }]}>{stamp(at)}</Text>
      <View style={styles.eventText}>
        <Text style={[styles.eventTitle, { color }]}>{title}</Text>
        {detail ? (
          <Text style={[styles.eventDetail, { color: colors.muted }]}>{detail}</Text>
        ) : null}
      </View>
    </View>
  );
}

/** How a conversation stands. `quiet` says only that nothing printed: never that it waits. */
function sessionWords(session: SessionSummary): string {
  switch (session.process) {
    case "running":
      return session.terminal ? terminalWords(session.terminal) : "Running";
    case "ended":
      return session.exitCode === null || session.exitCode === 0
        ? "Ended"
        : `Ended with ${session.exitCode}`;
    case "gone":
      return "Its process is gone";
    default:
      return "The computer cannot tell";
  }
}

function terminalWords(terminal: TerminalSummary): string {
  switch (terminal.activity) {
    case "starting":
      return "Starting";
    case "busy":
      return "Printing";
    case "quiet":
      return `Nothing printed for ${duration(terminal.idleMs)}`;
    case "exited":
      return terminal.exitCode === null ? "Exited" : `Exited with ${terminal.exitCode}`;
  }
}

function duration(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return `${Math.max(1, Math.round(ms / 1000))} s`;
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h`;
}

function purposeWords(purpose: TerminalPurpose, program: string): string {
  switch (purpose) {
    case "shell":
      return "Shell";
    case "runCommand":
      return `Run command: ${program}`;
    default:
      return program;
  }
}

const styles = StyleSheet.create({
  mark: { width: 10, height: 10, borderRadius: 2 },
  event: { flexDirection: "row", gap: 12, paddingHorizontal: 16, paddingVertical: 10 },
  eventTime: { fontFamily: FONTS.mono, fontSize: 12, lineHeight: 18 },
  eventText: { flex: 1, gap: 2 },
  eventTitle: { fontFamily: FONTS.mono, fontSize: 13, lineHeight: 18 },
  eventDetail: { fontFamily: FONTS.regular, fontSize: 13, lineHeight: 18 },
});
