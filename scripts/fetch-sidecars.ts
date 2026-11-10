import { $ } from "bun";
import { chmod, copyFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const NODE_VERSION = "v24.22.2";

const root = resolve(import.meta.dir, "..");
const binDir = join(root, "src-tauri", "binaries");
const resDir = join(root, "src-tauri", "resources");
const serverUrlFile = resolve(root, "server-url.txt");
const force = process.argv.includes("--force");

const nodePlatforms: Record<string, { dist: string; ext: "tar.gz" | "tar.xz" | "zip" }> = {
  "aarch64-apple-darwin": { dist: "darwin-arm64", ext: "tar.gz" },
  "x86_64-apple-darwin": { dist: "darwin-x64", ext: "tar.gz" },
  "x86_64-pc-windows-msvc": { dist: "win-x64", ext: "zip" },
  "aarch64-pc-windows-msvc": { dist: "win-arm64", ext: "zip" },
  "x86_64-unknown-linux-gnu": { dist: "linux-x64", ext: "tar.xz" },
  "aarch64-unknown-linux-gnu": { dist: "linux-arm64", ext: "tar.xz" },
};

async function hostTriple(): Promise<string> {
  const fromArg = process.argv.find((a) => a.startsWith("--target="));
  if (fromArg) return fromArg.slice("--target=".length);
  const info = await $`rustc -vV`.text();
  const host = info.split("\n").find((l) => l.startsWith("host: "));
  if (!host) throw new Error("Could not read host triple from `rustc -vV`");
  return host.slice("host: ".length).trim();
}

async function download(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`GET ${url} failed: ${res.status}`);
  return res.arrayBuffer();
}

function sha256(data: ArrayBuffer): string {
  return new Bun.CryptoHasher("sha256").update(data).digest("hex");
}

async function fetchNode(triple: string, exe: string) {
  const out = join(binDir, `node-${triple}${exe}`);
  if (existsSync(out) && !force) return console.log(`node: already present (${out})`);

  const platform = nodePlatforms[triple];
  if (!platform) throw new Error(`No Node build mapping for target ${triple}`);
  const name = `node-${NODE_VERSION}-${platform.dist}`;
  const archive = `${name}.${platform.ext}`;
  const base = `https://nodejs.org/dist/${NODE_VERSION}`;

  console.log(`node: downloading ${archive}`);
  const [data, sums] = await Promise.all([download(`${base}/${archive}`), download(`${base}/SHASUMS256.txt`)]);
  const expected = new TextDecoder()
    .decode(sums)
    .split("\n")
    .find((l) => l.endsWith(`  ${archive}`))
    ?.split(" ")[0];
  if (!expected || expected !== sha256(data)) throw new Error(`Checksum mismatch for ${archive}`);

  const work = await mkdtemp(join(tmpdir(), "stremio-node-"));
  try {
    const archivePath = join(work, archive);
    await Bun.write(archivePath, data);
    const member = exe ? `${name}/node.exe` : `${name}/bin/node`;
    await $`tar -xf ${archivePath} -C ${work} ${member}`;
    await copyFile(join(work, member), out);
    await chmod(out, 0o755);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
  console.log(`node: ${NODE_VERSION} -> ${out}`);
}

async function fetchServer() {
  const out = join(resDir, "server.js");
  if (existsSync(out) && !force) return console.log(`server.js: already present (${out})`);
  const url = (await Bun.file(serverUrlFile).text()).trim();
  console.log(`server.js: downloading ${url}`);
  await Bun.write(out, await download(url));
}

async function copyFfmpegTools(triple: string, exe: string) {
  const sourceDir = process.env.FFMPEG_DIR;
  for (const tool of ["ffmpeg", "ffprobe"]) {
    const out = join(binDir, `${tool}-${triple}${exe}`);
    if (existsSync(out) && !force) {
      console.log(`${tool}: already present (${out})`);
      continue;
    }
    const source = sourceDir ? join(sourceDir, `${tool}${exe}`) : Bun.which(tool);
    if (!source || !existsSync(source)) {
      throw new Error(`${tool} not found. Set FFMPEG_DIR to a folder with static ${tool}${exe} builds.`);
    }
    if (!sourceDir) console.warn(`${tool}: using ${source} from PATH; fine for dev, use a static build for releases`);
    await copyFile(source, out);
    await chmod(out, 0o755);
    console.log(`${tool}: ${source} -> ${out}`);
  }
}

const triple = await hostTriple();
const exe = triple.includes("windows") ? ".exe" : "";
await mkdir(binDir, { recursive: true });
await mkdir(resDir, { recursive: true });
await fetchNode(triple, exe);
await fetchServer();
await copyFfmpegTools(triple, exe);
