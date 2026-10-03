import { useRef, useState } from "react";
import { ContextMenu, type MenuItem } from "@/features/sidebar/ContextMenu";
import { useTasksStore, type Target } from "@/stores/tasks";
import type { Row } from "./rows";

const button =
  "h-7 rounded border border-line px-2.5 whitespace-nowrap hover:bg-raised disabled:opacity-40 disabled:hover:bg-transparent";

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
/** Each name once, whatever its capitals, in the order given. */
function distinct(names: string[]): string[] {
  return names.filter((name, index) => names.findIndex((other) => same(other, name)) === index);
}

/** A button that opens a menu under itself, and takes the focus back when the menu closes. */
function MenuButton({
  label,
  title,
  disabled,
  items,
  onOpen,
}: {
  label: string;
  title: string;
  disabled: boolean;
  items: MenuItem[];
  onOpen?: () => void;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const ref = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-haspopup="menu"
        aria-expanded={!!menu}
        disabled={disabled}
        title={title}
        onClick={(event) => {
          const box = event.currentTarget.getBoundingClientRect();
          setMenu(menu ? null : { x: box.left, y: box.bottom + 4 });
          if (!menu) onOpen?.();
        }}
        className={button}
      >
        {label} <span aria-hidden>▾</span>
      </button>
      {menu && (
        <ContextMenu
          at={menu}
          onClose={() => {
            setMenu(null);
            ref.current?.focus();
          }}
          items={items}
        />
      )}
    </>
  );
}

/**
 * What can be done to a task on its source: close it or reopen it, label it, assign it.
 *
 * Closing and reopening are asked about first, because they tell other people something.
 * Labels and assignees are applied as they are ticked: each is undone by unticking it.
 */
export function Manage({ row }: { row: Row }) {
  const { task, project, viewer } = row;
  const busy = useTasksStore((s) => s.busy !== null);
  const choices = useTasksStore((s) => s.choices[project.id]);
  const target: Target = { key: row.key, projectId: project.id, task };
  const store = useTasksStore.getState();
  const loadChoices = () => void store.loadChoices(project.id);

  const has = (names: string[], name: string) => names.some((other) => same(other, name));
  const mine = task.labels.map((label) => label.name);
  // What the task already has is always offered, so it can be taken off even when the
  // project's own list could not be read.
  const labels = distinct([...mine, ...(choices?.labels.map((label) => label.name) ?? [])]);
  const people = distinct([
    ...(viewer ? [viewer] : []),
    ...task.assignees,
    ...(choices?.assignees ?? []),
  ]);

  return (
    <>
      {task.state === "open" ? (
        <MenuButton
          label="Close"
          title="Close it, saying why"
          disabled={busy}
          items={[
            { label: "Close as completed", onSelect: () => void store.close(target, "completed") },
            {
              label: "Close as not planned",
              onSelect: () => void store.close(target, "notPlanned"),
            },
          ]}
        />
      ) : (
        <button
          type="button"
          disabled={busy}
          onClick={() => void store.reopen(target)}
          className={button}
        >
          Reopen
        </button>
      )}
      <MenuButton
        label="Labels"
        title="Add or remove a label"
        disabled={busy}
        onOpen={loadChoices}
        items={
          labels.length === 0
            ? [{ label: "This repository has no labels", disabled: true, onSelect: () => {} }]
            : labels.map((name) => {
                const on = has(mine, name);
                return {
                  label: name,
                  checked: on,
                  onSelect: () =>
                    void store.edit(
                      target,
                      on ? { removeLabels: [name] } : { addLabels: [name] },
                      on ? `Removed ${name} from ${task.key}.` : `Labelled ${task.key} ${name}.`,
                    ),
                };
              })
        }
      />
      <MenuButton
        label="Assignees"
        title="Assign it, or take someone off it"
        disabled={busy}
        onOpen={loadChoices}
        items={
          people.length === 0
            ? [{ label: "Nobody can be assigned", disabled: true, onSelect: () => {} }]
            : people.map((login) => {
                const on = has(task.assignees, login);
                const you = !!viewer && same(login, viewer);
                return {
                  label: you ? `${login} (you)` : login,
                  checked: on,
                  onSelect: () =>
                    void store.edit(
                      target,
                      on ? { removeAssignees: [login] } : { addAssignees: [login] },
                      on ? `Took ${login} off ${task.key}.` : `Assigned ${task.key} to ${login}.`,
                    ),
                };
              })
        }
      />
    </>
  );
}

/** A comment on the task. Sending is the confirmation: nothing is posted any other way. */
export function Reply({ target }: { target: Target }) {
  const [text, setText] = useState("");
  const busy = useTasksStore((s) => s.busy !== null);
  const ready = text.trim() !== "" && !busy;
  const send = async () => {
    if (!ready) return;
    const sent = await useTasksStore.getState().comment(target, text.trim());
    if (sent) setText("");
  };
  return (
    <form
      aria-label="Reply"
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
      className="mt-3"
    >
      <textarea
        aria-label="Your comment"
        placeholder="Write a comment…"
        value={text}
        rows={text.includes("\n") ? 5 : 2}
        disabled={busy}
        spellCheck
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          }
        }}
        className="w-full resize-y rounded border border-line bg-canvas px-2 py-1.5 outline-none select-text focus:border-accent disabled:opacity-50"
      />
      <div className="mt-1.5 flex items-center justify-end gap-3 text-[11px] text-ink-faint">
        <span>Markdown, as on GitHub. Ctrl+Enter or ⌘Enter sends.</span>
        <button
          type="submit"
          disabled={!ready}
          className="rounded bg-accent px-3 py-1 font-medium text-canvas disabled:opacity-40"
        >
          Comment
        </button>
      </div>
    </form>
  );
}
