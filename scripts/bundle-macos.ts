import { $ } from "bun";
import { chmod, copyFile, mkdir, mkdtemp, readdir, realpath, rm, symlink } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
const config = await Bun.file(join(root, "src-tauri", "tauri.conf.json")).json();
const productName: string = config.productName;
const version: string = config.version;

const bundleDir = join(root, "src-tauri", "target", "release", "bundle");
const app = join(bundleDir, "macos", `${productName}.app`);
const macosDir = join(app, "Contents", "MacOS");
const frameworksDir = join(app, "Contents", "Frameworks");
const dmg = join(bundleDir, "dmg", `${productName}_${version}_${process.arch}.dmg`);

const isSystemLibrary = (path: string) => path.startsWith("/usr/lib/") || path.startsWith("/System/");

async function linkedLibraries(binary: string): Promise<string[]> {
  const lines = (await $`otool -L ${binary}`.text()).split("\n").slice(1);
  return lines.map((l) => l.trim().split(" (")[0]).filter(Boolean);
}

async function rpaths(binary: string): Promise<string[]> {
  const lines = (await $`otool -l ${binary}`.text()).split("\n");
  const found: string[] = [];
  lines.forEach((line, i) => {
    if (line.includes("cmd LC_RPATH")) {
      const path = lines[i + 2]?.trim().match(/^path (.+) \(offset/)?.[1];
      if (path) found.push(path);
    }
  });
  return found;
}

async function resolveReference(reference: string, from: string, fromOriginal: string): Promise<string | null> {
  if (!reference.startsWith("@")) return existsSync(reference) ? realpath(reference) : null;
  const name = reference.replace(/^@(rpath|loader_path|executable_path)\//, "");
  const searchDirs = [dirname(fromOriginal)];
  for (const rpath of await rpaths(from)) {
    searchDirs.push(rpath.replace("@loader_path", dirname(fromOriginal)));
  }
  searchDirs.push("/opt/homebrew/lib", "/usr/local/lib");
  for (const dir of searchDirs) {
    const candidate = join(dir, name);
    if (existsSync(candidate)) return realpath(candidate);
  }
  return null;
}

async function isMachO(path: string): Promise<boolean> {
  return (await $`file -b ${path}`.text()).includes("Mach-O");
}

async function bundleLibraries() {
  await mkdir(frameworksDir, { recursive: true });
  const nameByRealPath = new Map<string, string>();
  const originalByBundled = new Map<string, string>();
  const queue: string[] = [];

  for (const entry of await readdir(macosDir)) {
    const path = join(macosDir, entry);
    if (await isMachO(path)) {
      queue.push(path);
      originalByBundled.set(path, path);
    }
  }

  while (queue.length) {
    const binary = queue.shift()!;
    const original = originalByBundled.get(binary)!;
    const inFrameworks = dirname(binary) === frameworksDir;
    const changes: string[] = [];

    for (const reference of await linkedLibraries(binary)) {
      if (isSystemLibrary(reference) || reference.startsWith("@executable_path/../Frameworks/")) continue;
      if (inFrameworks && reference.startsWith("@loader_path/") && existsSync(join(frameworksDir, basename(reference)))) continue;
      const source = await resolveReference(reference, binary, original);
      if (!source) throw new Error(`${basename(binary)} needs ${reference}, which was not found`);

      let name = nameByRealPath.get(source);
      if (!name) {
        name = basename(reference);
        nameByRealPath.set(source, name);
        const bundled = join(frameworksDir, name);
        await copyFile(source, bundled);
        await chmod(bundled, 0o755);
        await $`install_name_tool -id ${`@rpath/${name}`} ${bundled}`.quiet();
        originalByBundled.set(bundled, source);
        queue.push(bundled);
      }
      const target = inFrameworks ? `@loader_path/${name}` : `@executable_path/../Frameworks/${name}`;
      if (reference !== target) changes.push("-change", reference, target);
    }

    if (changes.length) await $`install_name_tool ${changes} ${binary}`.quiet();
  }
  console.log(`bundled ${nameByRealPath.size} libraries into Contents/Frameworks`);
}

async function signApp() {
  for (const entry of await readdir(frameworksDir)) {
    await $`codesign --force --sign - --timestamp=none ${join(frameworksDir, entry)}`.quiet();
  }
  for (const entry of await readdir(macosDir)) {
    const path = join(macosDir, entry);
    const signed = await $`codesign --verify ${path}`.nothrow().quiet();
    if (signed.exitCode !== 0) await $`codesign --force --sign - --timestamp=none ${path}`.quiet();
  }
  await $`codesign --force --sign - --timestamp=none ${app}`.quiet();
  await $`codesign --verify --deep --strict ${app}`;
  console.log("signed for local use (ad-hoc)");
}

async function checkNoExternalLibraries() {
  const leftovers: string[] = [];
  for (const dir of [macosDir, frameworksDir]) {
    for (const entry of await readdir(dir)) {
      const path = join(dir, entry);
      if (!(await isMachO(path))) continue;
      for (const reference of await linkedLibraries(path)) {
        if (reference.startsWith("/") && !isSystemLibrary(reference)) leftovers.push(`${entry} -> ${reference}`);
      }
    }
  }
  if (leftovers.length) throw new Error(`libraries still linked from outside the app:\n${leftovers.join("\n")}`);
  console.log("no links to libraries outside the app");
}

async function makeDmg() {
  const staging = await mkdtemp(join(tmpdir(), "stremio-dmg-"));
  try {
    await $`ditto ${app} ${join(staging, `${productName}.app`)}`;
    await symlink("/Applications", join(staging, "Applications"));
    await mkdir(dirname(dmg), { recursive: true });
    await rm(dmg, { force: true });
    await $`hdiutil create -volname ${productName} -srcfolder ${staging} -ov -format UDZO ${dmg}`.quiet();
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
  console.log(`disk image: ${dmg}`);
}

if (!existsSync(app)) throw new Error(`${app} not found; run \`bun run tauri build --bundles app\` first`);
await bundleLibraries();
await checkNoExternalLibraries();
await signApp();
await makeDmg();
