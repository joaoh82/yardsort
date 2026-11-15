// The path of a project's worktree workspace whose name holds a word, from `ys workspace list
// --json` on standard input; nothing, and exit 1, when there is none.
//
//   ys workspace list --project <p> --json | bun .yardsort/tests/support/workspace-path.ts <word> [branch]
//
// With `branch`, its branch instead of its path.

const [word, what] = process.argv.slice(2);
const all: { kind: string; name: string; path: string; branch: string | null }[] = JSON.parse(
  await Bun.stdin.text(),
);
const found = all.find((w) => w.kind === "worktree" && w.name.includes(word));
if (!found) process.exit(1);
console.log(what === "branch" ? found.branch : found.path);
