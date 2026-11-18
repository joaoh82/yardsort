// Read an account's export as its person would, for account-export.yaml: the file one client
// saved holds the account, the computers named, and the note, is readable by its owner only, and
// holds none of the sign-ins this run's computers keep.
//
//   bun .yardsort/tests/support/export-check.ts <run dir> <file> <note> <computer>…

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const [dir, file, note, ...computers] = process.argv.slice(2);
const path = join(dir, file);
const text = readFileSync(path, "utf8");
const exported = JSON.parse(text);

const fail = (why: string): never => {
  console.error(`${file}: ${why}`);
  process.exit(1);
};

if (exported.format !== 1) fail(`format ${exported.format}, not 1`);
if (!exported.account?.subject) fail("no account");
const names: string[] = exported.account.hosts.map((host: { name: string }) => host.name);
for (const computer of computers) if (!names.includes(computer)) fail(`no ${computer} in ${names}`);
const noted = exported.records.some(
  (record: { kind: string; body: { text?: string } }) =>
    record.kind === "note" && record.body.text === note,
);
if (!noted) fail(`no note "${note}"`);
if ((statSync(path).mode & 0o777) !== 0o600) fail("readable by others");

// Every kept sign-in: the app's, and the computer with no screen's.
const kept: string[] = [];
const gather = (at: string) => {
  let entries;
  try {
    entries = readdirSync(at, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const inner = join(at, entry.name);
    if (entry.isDirectory()) gather(inner);
    else kept.push(readFileSync(inner, "utf8").trim());
  }
};
gather(join(dir, "credentials"));
gather(join(dir, "host", "credentials"));
if (kept.length === 0) fail("no kept sign-in found to look for");
for (const secret of kept)
  if (secret.length >= 16 && text.includes(secret)) fail("holds a sign-in");

console.log(
  `${file}: ${exported.account.devices.length} devices, ${names.length} computers, ` +
    `${exported.records.length} records, the note, no sign-in`,
);
