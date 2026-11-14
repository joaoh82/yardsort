import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { project } from "@/test/fixtures";
import { LinkSyncedProjectDialog } from "./LinkSyncedProjectDialog";

const core = vi.hoisted(() => ({
  projectSyncOffers: vi.fn(),
  projectSyncLink: vi.fn(),
}));
vi.mock("@/lib/ipc", async (original) => ({
  ...(await original<typeof import("@/lib/ipc")>()),
  ipc: core,
}));

const linked = {
  sync: { state: "on", reached: false, removing: false, refused: null },
  syncedAt: null,
  problem: null,
} as const;
beforeEach(() => {
  vi.clearAllMocks();
  core.projectSyncOffers.mockResolvedValue([
    { id: "s-match", name: "demo", remote: "github.com/acme/demo", matches: true },
    { id: "s-other", name: "scratch", remote: null, matches: false },
  ]);
  core.projectSyncLink.mockResolvedValue(linked);
});

const show = (props: { onClose?: () => void; onLinked?: () => void } = {}) =>
  render(
    <LinkSyncedProjectDialog
      project={project("demo")}
      onClose={props.onClose ?? (() => {})}
      onLinked={props.onLinked ?? (() => {})}
    />,
  );

it("lists the offers with their hints and picks none, not even the matching one", async () => {
  show();
  expect(screen.getByRole("dialog", { name: "Link “demo” to a synced project" })).toBeVisible();
  expect(screen.getByText("Asking the server…")).toBeInTheDocument();
  const match = await screen.findByRole("radio", { name: "demo" });
  const other = screen.getByRole("radio", { name: "scratch" });
  expect(match).not.toBeChecked();
  expect(other).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Link" })).toBeDisabled();
  const rows = within(screen.getByRole("list", { name: "Synced projects" })).getAllByRole(
    "listitem",
  );
  expect(rows[0]).toHaveTextContent("Same remote");
  expect(rows[0]).toHaveTextContent("github.com/acme/demo");
  expect(rows[1]).not.toHaveTextContent("Same remote");
  expect(rows[1]).toHaveTextContent("no remote");
  expect(screen.getByText(/Link only to the same repository/)).toBeInTheDocument();
  expect(core.projectSyncOffers).toHaveBeenCalledWith("p-demo");
});

it("links the chosen one, then closes", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  const onLinked = vi.fn();
  show({ onClose, onLinked });
  await user.click(await screen.findByRole("radio", { name: "scratch" }));
  await user.click(screen.getByRole("button", { name: "Link" }));
  expect(core.projectSyncLink).toHaveBeenCalledWith("p-demo", "s-other");
  expect(onLinked).toHaveBeenCalledWith(linked);
  expect(onClose).toHaveBeenCalledOnce();
});

it("a refused link stays open with the reason", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  core.projectSyncLink.mockRejectedValue({ code: "remote", message: "Already linked" });
  show({ onClose });
  await user.click(await screen.findByRole("radio", { name: "demo" }));
  await user.click(screen.getByRole("button", { name: "Link" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Already linked");
  expect(onClose).not.toHaveBeenCalled();
});

it("says when there is nothing to link", async () => {
  core.projectSyncOffers.mockResolvedValue([]);
  show();
  expect(
    await screen.findByText(/No other computer syncs a project you could link here/),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Link" })).not.toBeInTheDocument();
});

it("shows a server it could not reach", async () => {
  core.projectSyncOffers.mockRejectedValue({
    code: "remote",
    message: "The server is unreachable",
  });
  show();
  expect(await screen.findByRole("alert")).toHaveTextContent("The server is unreachable");
  expect(screen.queryByText("Asking the server…")).not.toBeInTheDocument();
});

it("Escape closes it", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  show({ onClose });
  await screen.findByRole("radio", { name: "demo" });
  await user.keyboard("{Escape}");
  expect(onClose).toHaveBeenCalledOnce();
  expect(core.projectSyncLink).not.toHaveBeenCalled();
});
