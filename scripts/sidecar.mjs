// Build `ys` in release mode and put it where Tauri looks for the app's sidecar:
// `src-tauri/binaries/ys-<target triple>[.exe]`. `src-tauri/tauri.bundle.conf.json` names it, and
// every bundle then carries `ys` next to the app's own executable (see `src-tauri/src/ys.rs`).
//
//   bun scripts/sidecar.mjs              for this machine
//   bun scripts/sidecar.mjs --universal  macOS: both architectures, and the lipo'd pair
//
// Plain JavaScript on Node's APIs, so it runs the same under bun on all three systems.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const out = join(root, "src-tauri", "binaries");
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : join(root, "target");
const exe = process.platform === "win32" ? ".exe" : "";

const run = (program, args) => execFileSync(program, args, { cwd: root, stdio: "inherit" });

/** Build for `triple`, or for this machine when it is null. Returns the binary's path. */
function build(triple) {
  run("cargo", [
    "build",
    "--release",
    "-p",
    "yardsort-cli",
    ...(triple ? ["--target", triple] : []),
  ]);
  return join(targetDir, ...(triple ? [triple] : []), "release", `ys${exe}`);
}

mkdirSync(out, { recursive: true });
if (process.argv.includes("--universal")) {
  const parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"].map((triple) => {
    const built = build(triple);
    copyFileSync(built, join(out, `ys-${triple}`));
    return built;
  });
  const universal = join(out, "ys-universal-apple-darwin");
  run("lipo", ["-create", "-output", universal, ...parts]);
  run("lipo", ["-info", universal]);
} else {
  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (\S+)$/m)?.[1];
  if (!host) throw new Error("rustc -vV did not name the host triple");
  const dest = join(out, `ys-${host}${exe}`);
  copyFileSync(build(null), dest);
  console.log(`sidecar: ${dest}`);
}
