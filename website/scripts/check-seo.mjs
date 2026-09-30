// Check what crawlers actually receive, including Next.js metadata inheritance and export.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const out = path.resolve(import.meta.dirname, "../out");
const origin = "https://www.yardsort.sh";
const decode = (value) =>
  value.replace(
    /&(amp|quot|lt|gt|#x27|#39);/g,
    (_, entity) => ({ amp: "&", quot: '"', lt: "<", gt: ">", "#x27": "'", "#39": "'" })[entity],
  );
const attrs = (tag) =>
  Object.fromEntries([...tag.matchAll(/([\w:-]+)="([^"]*)"/g)].map((m) => [m[1], decode(m[2])]));

function htmlFiles(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? htmlFiles(file) : file.endsWith(".html") ? [file] : [];
  });
}

const titles = new Set();
const descriptions = new Set();
const canonicals = [];
for (const file of htmlFiles(out)) {
  const relative = path.relative(out, file).split(path.sep).join("/");
  const html = fs.readFileSync(file, "utf8");
  const metas = [...html.matchAll(/<meta\s[^>]*>/g)].map((m) => attrs(m[0]));
  function meta(key) {
    const matches = metas.filter((m) => m.name === key || m.property === key);
    assert.equal(matches.length, 1, `${relative}: expected one ${key}`);
    return matches[0].content;
  }
  if (["404.html", "404/index.html", "_not-found/index.html"].includes(relative)) {
    assert.match(meta("robots"), /noindex/, `${relative}: error page must not be indexed`);
    assert(!html.includes('rel="canonical"'), `${relative}: must not inherit a canonical`);
    continue;
  }
  assert(relative.endsWith("index.html"), `Unexpected exported page: ${relative}`);
  const canonical = `${origin}/${relative.slice(0, -"index.html".length)}`;
  const links = [...html.matchAll(/<link\s[^>]*>/g)].map((m) => attrs(m[0]));
  assert.deepEqual(
    links.filter((link) => link.rel === "canonical").map((link) => link.href),
    [canonical],
  );
  canonicals.push(canonical);
  assert(!metas.some((m) => /noindex/.test(m.content ?? "")), `${relative}: unexpected noindex`);
  const title = decode(html.match(/<title>([^<]+)<\/title>/)?.[1] ?? "");
  const description = meta("description");
  assert(title && description, `${relative}: missing title or description`);
  assert(!titles.has(title), `${relative}: duplicate title`);
  assert(!descriptions.has(description), `${relative}: duplicate description`);
  titles.add(title);
  descriptions.add(description);
  assert.equal(meta("og:title"), title);
  assert.equal(meta("twitter:title"), title);
  assert.equal(meta("og:description"), description);
  assert.equal(meta("twitter:description"), description);
  assert.equal(meta("og:url"), canonical);
  assert.equal(meta("og:site_name"), "Yardsort");
  assert.equal(meta("og:type"), "website");
  assert.equal(meta("twitter:card"), "summary_large_image");
  const image = new URL(meta("og:image"));
  assert.equal(image.origin, origin);
  assert.equal(meta("twitter:image"), image.href);
  assert(meta("og:image:alt"));
  assert.equal(meta("twitter:image:alt"), meta("og:image:alt"));
  assert.equal(meta("og:image:width"), "1200");
  assert.equal(meta("og:image:height"), "630");
  assert.equal(meta("og:image:type"), "image/png");
  const png = fs.readFileSync(path.join(out, image.pathname));
  assert.equal(png.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  assert.equal(png.readUInt32BE(16), 1200);
  assert.equal(png.readUInt32BE(20), 630);
}

const sitemap = fs.readFileSync(path.join(out, "sitemap.xml"), "utf8");
const urls = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => decode(m[1]));
assert(canonicals.length > 2, "Expected exported documentation pages");
assert.deepEqual(
  urls.sort(),
  canonicals.sort(),
  "Sitemap must cover exactly the exported public pages",
);
assert(!urls.includes(`${origin}/docs/quick-start/`), "Quick start lives at /docs/");
const robots = fs.readFileSync(path.join(out, "robots.txt"), "utf8");
assert.match(robots, /^User-Agent: \*$/m);
assert.match(robots, /^Allow: \/$/m);
assert(!/^Disallow:\s*\//m.test(robots));
assert(robots.includes(`Sitemap: ${origin}/sitemap.xml`));
console.log(
  `SEO export checks passed: ${canonicals.length} pages, sitemap, robots and social image.`,
);
