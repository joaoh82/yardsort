import ReactDOM from "react-dom/client";
import "./styles.css";
import { project, task, taskDetail, tasksOf, worktree } from "@/test/fixtures";
import { useProjectsStore } from "@/stores/projects";
import { detailStamp, useTasksStore } from "@/stores/tasks";
import { TasksView } from "@/features/tasks/TasksView";
import { NewTaskDialog } from "@/features/tasks/NewTaskDialog";
import { rowKey } from "@/features/tasks/rows";

const base = project("yardsort");
const ago = (h: number) => new Date(Date.now() - h * 3600e3).toISOString();
const drive = task(92, {
  title: "Worktrees on a network drive are slow to create",
  author: "grace",
  labels: [
    { name: "bug", color: "d73a4a" },
    { name: "windows", color: "0075ca" },
  ],
  assignees: ["ada"],
  comments: 2,
  needsAnswer: true,
  updatedAt: ago(1),
  createdAt: ago(48),
});
const ws = {
  ...worktree("yardsort", "92-worktrees-network-drive"),
  tasks: [
    {
      source: "github" as const,
      repo: "github.com/demo/app",
      key: "#92",
      url: drive.url,
      title: drive.title,
    },
  ],
};
const alpha = { ...base, workspaces: [...base.workspaces, ws] };
const tasks = [
  drive,
  task(91, {
    title: "Git repositories as first-class citizens",
    author: "ken",
    labels: [{ name: "enhancement", color: "a2eeef" }],
    needsAnswer: true,
    updatedAt: ago(2),
  }),
  task(88, {
    title: "Document the daemon's socket",
    author: "ada",
    comments: 2,
    updatedAt: ago(24),
  }),
];
const params = new URLSearchParams(location.search);
useProjectsStore.setState({ projects: [alpha], loaded: true, tasksOpen: true, ui: {} });
const key = rowKey(alpha.id, drive.key);
const choices = {
  labels: [
    "bug",
    "documentation",
    "enhancement",
    "good first issue",
    "help wanted",
    "question",
    "windows",
  ].map((name) => ({ name, color: "888888" })),
  assignees: ["ada", "grace"],
};
useTasksStore.setState({
  byProject: { [alpha.id]: tasksOf(tasks) },
  selected: key,
  notice: params.has("notice") ? "Labelled #92 windows." : null,
  choices: { [alpha.id]: choices },
  details: {
    [key]: {
      loading: false,
      error: null,
      stamp: detailStamp(drive),
      detail: taskDetail(drive, {
        body: "Creating a workspace on an SMB share takes **over a minute**.",
        comments: [
          {
            author: "ada",
            createdAt: ago(24),
            body: "Which filesystem is the share?",
            url: "https://example.com/2",
            hidden: null,
            bot: false,
            maintainer: true,
          },
          {
            author: "grace",
            createdAt: ago(1),
            body: "NTFS over SMB 3.",
            url: "https://example.com/3",
            hidden: null,
            bot: false,
            maintainer: false,
          },
        ],
      }),
    },
  },
});
ReactDOM.createRoot(document.getElementById("root")!).render(
  <div className="h-full bg-canvas text-ink text-[13px]">
    <TasksView />
    {params.has("dialog") && (
      <NewTaskDialog projects={[alpha]} initial={alpha.id} onClose={() => {}} />
    )}
  </div>,
);
