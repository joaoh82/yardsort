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
const textContent = (html) =>
  decode(
    html
      .replace(/<!--[\s\S]*?-->/g, "")
      .replace(/<[^>]*>/g, " ")
      .replace(/\s+/g, " ")
      .trim(),
  );
const answerSections = [];
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

  const schemas = [
    ...html.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g),
  ].map((match) => JSON.parse(match[1]));
  for (const schema of schemas) assert.equal(schema["@context"], "https://schema.org");
  if (relative === "index.html") {
    assert.equal(schemas.length, 1);
    const graph = schemas[0]["@graph"];
    assert.deepEqual(
      graph.map((node) => node["@type"]),
      ["WebSite", "SoftwareApplication"],
    );
    const [site, app] = graph;
    assert.equal(site.url, canonical);
    assert.equal(app.url, canonical);
    assert.equal(site.mainEntity["@id"], app["@id"]);
    assert.equal(app.name, "Yardsort");
    assert.equal(app.description, description);
    assert.equal(app.applicationCategory, "DeveloperApplication");
    assert.equal(app.operatingSystem, "Linux, macOS, Windows");
    const version = JSON.parse(
      fs.readFileSync(path.join(out, "../../src-tauri/tauri.conf.json"), "utf8"),
    ).version;
    assert.equal(app.softwareVersion, version);
    assert(
      textContent(html.replace(/<script\b[\s\S]*?<\/script>/g, "")).includes(`v${version}`),
      "Schema version must also be visible",
    );
    assert.equal(app.offers.price, 0);
    assert.equal(app.license, "https://github.com/joaoh82/yardsort/blob/main/LICENSE");
    assert(!app.review && !app.aggregateRating, "Do not invent reviews or ratings");
    const questions = html.match(/<section id="questions"[\s\S]*?<\/section>/)?.[0];
    assert(questions, "Homepage answers must be present in static HTML");
    answerSections.push(
      [...questions.matchAll(/<h3[^>]*>(.*?)<\/h3>([\s\S]*?)(?=<h3|<p class="mt-6)/g)].map(
        (match) => textContent(match[1] + match[2]),
      ),
    );
  } else if (relative.startsWith("docs/")) {
    assert.equal(schemas.length, 1, `${relative}: expected breadcrumb schema`);
    assert.equal(schemas[0]["@type"], "BreadcrumbList");
    const items = schemas[0].itemListElement;
    assert.equal(items.length, relative === "docs/index.html" ? 2 : 3);
    const nav = html.match(/<nav aria-label="Breadcrumb"[^>]*>([\s\S]*?)<\/nav>/)?.[1];
    assert(nav, `${relative}: missing visible breadcrumbs`);
    const visible = [...nav.matchAll(/<(a|span)\b([^>]*)>(.*?)<\/(?:a|span)>/g)].filter(
      (match) => match[1] === "a" || attrs(match[2])["aria-current"] === "page",
    );
    assert.equal(visible.length, items.length);
    items.forEach((item, index) => {
      assert.equal(item["@type"], "ListItem");
      assert.equal(item.position, index + 1);
      assert.equal(item.name, textContent(visible[index][3]));
      const href = attrs(visible[index][2]).href;
      assert.equal(item.item, href ? new URL(href, origin).href : canonical);
    });
    assert.equal(items.at(-1).item, canonical);
    assert.equal(new Set(items.map((item) => item.item)).size, items.length);
    if (relative === "docs/guide/questions/index.html") {
      const prose = html.match(/<div class="prose mt-6 max-w-none">([\s\S]*?)<\/div>/)?.[1];
      assert(prose);
      answerSections.push(
        [...prose.matchAll(/<h2[^>]*>(.*?)<\/h2>([\s\S]*?)(?=<h2|$)/g)].map((match) =>
          textContent(match[1] + match[2]),
        ),
      );
    }
  } else {
    assert.equal(schemas.length, 0, "Product schema belongs on the homepage");
  }
}

assert.equal(answerSections.length, 2, "Expected homepage and guide answers");
assert.equal(answerSections[0].length, 7, "Expected all seven product questions");
assert.deepEqual(answerSections[0], answerSections[1], "Homepage and guide answers must agree");

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
