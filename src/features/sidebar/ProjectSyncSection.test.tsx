import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { project } from "@/test/fixtures";
import { useLayoutStore } from "@/stores/layout";
import type { ProjectSyncStatus } from "@/lib/ipc";
import { ProjectSyncSection } from "./ProjectSyncSection";

const core = vi.hoisted(() => ({
  projectSyncGet: vi.fn(),
  projectSyncOn: vi.fn(),
  projectSyncOffers: vi.fn(),
  projectSyncLink: vi.fn(),
  projectSyncStop: vi.fn(),
  projectSyncRemove: vi.fn(),
  onSyncChanged: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

const off = (remote: string | null = "github.com/acme/demo"): ProjectSyncStatus => ({
  sync: { state: "off", preview: { name: "demo", remote, notes: 2 } },
  syncedAt: null,
  problem: null,
});
const on = (
  fields: Partial<{ reached: boolean; removing: boolean; refused: string | null }> = {},
  rest: Partial<ProjectSyncStatus> = {},
): ProjectSyncStatus => ({
  sync: { state: "on", reached: true, removing: false, refused: null, ...fields },
  syncedAt: null,
  problem: null,
  ...rest,
});

let changed: (() => void) | null = null;
const unlisten = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  changed = null;
  core.onSyncChanged.mockImplementation((handler: () => void) => {
    changed = handler;
    return Promise.resolve(unlisten);
  });
});

const show = () => render(<ProjectSyncSection project={project("demo")} />);

it("says Remote is needed, and its link opens Settings → Remote after the dialog closes", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue({
    sync: { state: "remoteOff" },
    syncedAt: null,
    problem: null,
  });
  const onLeave = vi.fn();
  render(<ProjectSyncSection project={project("demo")} onLeave={onLeave} />);
  expect(await screen.findByText(/Sync needs Remote/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Settings → Remote" }));
  expect(onLeave).toHaveBeenCalledOnce();
  expect(useLayoutStore.getState().settingsOpen).toBe(true);
  expect(useLayoutStore.getState().settingsSection).toBe("remote");
});

it("shows exactly what turning sync on sends, and that the server reads it", async () => {
  core.projectSyncGet.mockResolvedValue(off());
  show();
  const preview = await screen.findByLabelText("What turning sync on sends");
  const values = within(preview).getAllByRole("definition");
  expect(values.map((value) => value.textContent)).toEqual([
    "demo",
    "github.com/acme/demo",
    "2 approved, each with its text and history",
  ]);
  expect(screen.getByText(/Never its files, path or terminals/)).toBeInTheDocument();
  expect(
    screen.getByText("The server keeps these in plain text, readable by whoever runs it."),
  ).toBeInTheDocument();
});

it("a project with no remote says so in the preview", async () => {
  core.projectSyncGet.mockResolvedValue(off(null));
  show();
  const preview = await screen.findByLabelText("What turning sync on sends");
  expect(within(preview).getAllByRole("definition")[1]).toHaveTextContent("none");
});

it("Turn on sync turns it on and shows where it stands", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue(off());
  core.projectSyncOn.mockResolvedValue(on({ reached: false }));
  show();
  await user.click(await screen.findByRole("button", { name: "Turn on sync" }));
  expect(core.projectSyncOn).toHaveBeenCalledWith("p-demo");
  expect(await screen.findByText("Waiting to reach the server…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Turn on sync" })).not.toBeInTheDocument();
});

it("says how an on project stands and how the last pass went", async () => {
  core.projectSyncGet.mockResolvedValue(on({}, { syncedAt: Date.now() - 5 * 60_000 }));
  const { unmount } = show();
  expect(await screen.findByText("Syncing with your other computers.")).toBeInTheDocument();
  expect(screen.getByText("Last synced 5 min ago")).toBeInTheDocument();
  unmount();

  core.projectSyncGet.mockResolvedValue(
    on({}, { syncedAt: Date.now(), problem: "The server could not be reached" }),
  );
  const second = show();
  expect(
    await screen.findByText("Last attempt failed: The server could not be reached"),
  ).toBeInTheDocument();
  expect(screen.queryByText(/Last synced/)).not.toBeInTheDocument();
  second.unmount();

  core.projectSyncGet.mockResolvedValue(on({ refused: "Too many projects" }));
  const third = show();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The server refused it: Too many projects",
  );
  third.unmount();

  core.projectSyncGet.mockResolvedValue(on({ removing: true }));
  show();
  expect(await screen.findByText("Being removed from the server…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Remove from the server…" })).not.toBeInTheDocument();
});

it("Stop syncing stops it here", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue(on());
  core.projectSyncStop.mockResolvedValue(off());
  show();
  await user.click(await screen.findByRole("button", { name: "Stop syncing" }));
  expect(core.projectSyncStop).toHaveBeenCalledWith("p-demo");
  expect(await screen.findByRole("button", { name: "Turn on sync" })).toBeInTheDocument();
});

it("removing from the server asks first, and Keep it leaves it there", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue(on());
  core.projectSyncRemove.mockResolvedValue(on({ removing: true }));
  show();
  await user.click(await screen.findByRole("button", { name: "Remove from the server…" }));
  expect(screen.getByText(/removes the project’s records from the server/)).toBeInTheDocument();
  const keep = screen.getByRole("button", { name: "Keep it" });
  expect(keep).toHaveFocus();
  await user.click(keep);
  expect(core.projectSyncRemove).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "Keep it" })).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Remove from the server…" }));
  await user.click(screen.getByRole("button", { name: "Remove from the server" }));
  expect(core.projectSyncRemove).toHaveBeenCalledWith("p-demo");
  expect(await screen.findByText("Being removed from the server…")).toBeInTheDocument();
});

it("shows a failed call, and keeps the buttons", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue(on());
  core.projectSyncStop.mockRejectedValue({ code: "remote", message: "The server is unreachable" });
  show();
  await user.click(await screen.findByRole("button", { name: "Stop syncing" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("The server is unreachable");
  expect(screen.getByRole("button", { name: "Stop syncing" })).toBeEnabled();
});

it("reads it again when sync changes, and stops listening when closed", async () => {
  core.projectSyncGet.mockResolvedValueOnce(on({ reached: false }));
  const { unmount } = show();
  expect(await screen.findByText("Waiting to reach the server…")).toBeInTheDocument();
  core.projectSyncGet.mockResolvedValueOnce(on({}, { syncedAt: Date.now() }));
  act(() => changed?.());
  expect(await screen.findByText("Syncing with your other computers.")).toBeInTheDocument();
  expect(screen.getByText("Last synced just now")).toBeInTheDocument();
  unmount();
  await waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
});

it("a link made in the dialog shows at once", async () => {
  const user = userEvent.setup();
  core.projectSyncGet.mockResolvedValue(off());
  core.projectSyncOffers.mockResolvedValue([
    { id: "s-1", name: "demo", remote: "github.com/acme/demo", matches: true },
  ]);
  core.projectSyncLink.mockResolvedValue(on({ reached: false }));
  show();
  await user.click(await screen.findByRole("button", { name: "Link to a synced project…" }));
  const dialog = await screen.findByRole("dialog");
  await user.click(await within(dialog).findByRole("radio", { name: "demo" }));
  await user.click(within(dialog).getByRole("button", { name: "Link" }));
  expect(core.projectSyncLink).toHaveBeenCalledWith("p-demo", "s-1");
  expect(await screen.findByText("Waiting to reach the server…")).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
