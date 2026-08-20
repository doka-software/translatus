// Regenerates THIRD-PARTY-NPM.md from package-lock.json.
//
// The Rust side has cargo-about; this is the npm equivalent, kept deliberately
// dependency-free so that generating the licence inventory does not itself pull
// in packages that would need inventorying.
//
//   node tools/gen-npm-licenses.mjs
//
// Output is deterministic (stable sort, no timestamps) so a regeneration with
// no dependency change produces no diff.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const lock = JSON.parse(readFileSync(path.join(root, "package-lock.json"), "utf8"));
const pkg = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8"));

const direct = new Set([
  ...Object.keys(pkg.dependencies || {}),
  ...Object.keys(pkg.devDependencies || {}),
]);

const rows = Object.entries(lock.packages || {})
  .filter(([p]) => p.startsWith("node_modules/"))
  .map(([p, meta]) => {
    const name = p.slice(p.lastIndexOf("node_modules/") + "node_modules/".length);
    return {
      name,
      version: meta.version ?? "?",
      license: meta.license ?? "UNKNOWN",
      direct: direct.has(name),
      optional: Boolean(meta.optional),
    };
  })
  .sort((a, b) => Number(b.direct) - Number(a.direct) || a.name.localeCompare(b.name));

const counts = new Map();
for (const r of rows) counts.set(r.license, (counts.get(r.license) ?? 0) + 1);

const out = [];
out.push("# Third-party npm packages — subscription sidecar\n\n");
out.push(
  "Inventory of the npm dependency tree for `apps/subscription-kit`, generated\n" +
    "from `package-lock.json`. The Rust side is covered separately in\n" +
    "[`../../THIRD-PARTY-LICENSES.md`](../../THIRD-PARTY-LICENSES.md).\n\n" +
    "**This sidecar is optional and is not part of any released binary.** It is\n" +
    "installed by the user with `npm install` if they choose to use subscription\n" +
    "mode, so these packages are fetched from the npm registry at that point rather\n" +
    "than redistributed by this project. The list is provided so you can see what\n" +
    "you would be installing.\n\n" +
    "Regenerate after any dependency change:\n\n" +
    "```bash\n" +
    "node tools/gen-npm-licenses.mjs\n" +
    "```\n\n" +
    "Entries reading `SEE LICENSE IN ...` carry their terms in a file inside the\n" +
    "package itself; check those before redistributing anything that bundles them.\n"
);
out.push(`\n## Overview\n\n${rows.length} packages resolved.\n\n`);
out.push("| License | Packages |\n| --- | ---: |\n");
for (const [lic, n] of [...counts].sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0])))) {
  out.push(`| ${lic} | ${n} |\n`);
}
out.push("\n## Direct dependencies\n\n| Package | Version | License |\n| --- | --- | --- |\n");
for (const r of rows.filter((r) => r.direct)) {
  out.push(`| \`${r.name}\` | ${r.version} | ${r.license} |\n`);
}
out.push(
  "\n## Transitive and optional dependencies\n\n| Package | Version | License | Optional |\n| --- | --- | --- | --- |\n"
);
for (const r of rows.filter((r) => !r.direct)) {
  out.push(`| \`${r.name}\` | ${r.version} | ${r.license} | ${r.optional ? "yes" : ""} |\n`);
}

writeFileSync(path.join(root, "THIRD-PARTY-NPM.md"), out.join(""));
console.log(`THIRD-PARTY-NPM.md: ${rows.length} packages, ${counts.size} distinct licenses`);
