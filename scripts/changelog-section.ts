import { join, resolve } from "node:path";

const version = process.argv[2];
if (!version) throw new Error("usage: bun scripts/changelog-section.ts <version>");

const changelog = join(resolve(import.meta.dir, ".."), ".github", "release", "CHANGELOG.md");
const lines = (await Bun.file(changelog).text()).split("\n");
const start = lines.findIndex((line) => line.trim() === `## ${version}`);
const end = lines.findIndex((line, i) => i > start && line.startsWith("## "));
const section = start < 0 ? "" : lines.slice(start + 1, end < 0 ? undefined : end).join("\n").trim();

if (!section) {
  console.error(`.github/release/CHANGELOG.md has no entries for version ${version}.`);
  process.exit(1);
}
console.log(section);
