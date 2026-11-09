// Split the Linux app binary's debug information into a file of its own, before Tauri bundles it.
// `src-tauri/tauri.release.conf.json` runs this as its `beforeBundleCommand`.
//
// The release workflow builds the Linux app with line tables (CARGO_PROFILE_RELEASE_DEBUG). They
// make a crash's backtrace name the file and line it was in, and in the binary they would take
// the .deb from 15 MB to 53. So they move into `yardsort.debug` beside the binary, which the
// workflow attaches to the release. The binary keeps its symbol table, so that even without that
// file a backtrace names the functions, at a cost of half a megabyte compressed. It also keeps a
// `.gnu_debuglink` naming `yardsort.debug`, and its build ID, which gdb checks the file against.
//
// Elsewhere, or for a binary built without debug information, it does nothing.
//
// Plain JavaScript on Node's APIs, so it runs under bun like the other scripts.

import { execFileSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

if (process.platform !== "linux") process.exit(0);

const root = resolve(import.meta.dirname, "..");
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : join(root, "target");
const triple = process.env.TAURI_ENV_TARGET_TRIPLE;

// `--target` puts the build under the triple; without it, it is directly under the target
// directory. Whichever was built last is this build's.
const candidates = [
  ...(triple ? [join(targetDir, triple, "release", "yardsort")] : []),
  join(targetDir, "release", "yardsort"),
].filter((path) => existsSync(path));
if (candidates.length === 0) throw new Error(`split-debug: no yardsort binary in ${targetDir}`);
const binary = candidates.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];

const sections = execFileSync("readelf", ["--sections", "--wide", binary], { encoding: "utf8" });
if (!/\s\.debug_info\s/.test(sections)) {
  console.log(`split-debug: ${binary} has no debug information; nothing to split`);
  process.exit(0);
}

const debug = `${binary}.debug`;
const run = (args) => execFileSync("objcopy", args, { cwd: dirname(binary), stdio: "inherit" });
run(["--only-keep-debug", "--compress-debug-sections=zlib", binary, debug]);
// Relative to the binary's directory, so the link records only the file's name.
run(["--strip-debug", `--add-gnu-debuglink=${basename(debug)}`, binary]);

const mb = (path) => `${(statSync(path).size / 1048576).toFixed(1)} MB`;
console.log(
  `split-debug: ${binary} (${mb(binary)}), its debug information in ${debug} (${mb(debug)})`,
);
