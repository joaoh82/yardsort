// Copies the repo's docs/images into public/ so the docs pages can show them. Runs before dev and
// build; the copy is git-ignored because docs/images is the source.
import fs from "node:fs";
import path from "node:path";

const from = path.resolve(import.meta.dirname, "..", "..", "docs", "images");
const to = path.resolve(import.meta.dirname, "..", "public", "docs-images");
fs.rmSync(to, { recursive: true, force: true });
fs.cpSync(from, to, { recursive: true });
console.log(`docs images: ${fs.readdirSync(to).length} copied`);
