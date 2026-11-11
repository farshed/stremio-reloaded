import { $ } from "bun";
import { chmod, copyFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const NODE_VERSION = "v24.22.2";
const FFMPEG_BRANCH = "8.1";
const FFMPEG_RELEASES = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";

const root = resolve(import.meta.dir, "..");
const binDir = join(root, "src-tauri", "binaries");
const resDir = join(root, "src-tauri", "resources");
const serverUrlFile = resolve(root, "server-url.txt");
const force = process.argv.includes("--force");
// GNU tar from Git can shadow the Windows one, and it cannot read zip files.
const tar = process.platform === "win32" ? join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe") : "tar";
const downloadFfmpeg = process.argv.includes("--download-ffmpeg");

const nodePlatforms: Record<string, { dist: string; ext: "tar.gz" | "tar.xz" | "zip" }> = {
  "aarch64-apple-darwin": { dist: "darwin-arm64", ext: "tar.gz" },
  "x86_64-apple-darwin": { dist: "darwin-x64", ext: "tar.gz" },
  "x86_64-pc-windows-msvc": { dist: "win-x64", ext: "zip" },
  "aarch64-pc-windows-msvc": { dist: "win-arm64", ext: "zip" },
  "x86_64-unknown-linux-gnu": { dist: "linux-x64", ext: "tar.xz" },
  "aarch64-unknown-linux-gnu": { dist: "linux-arm64", ext: "tar.xz" },
};

const ffmpegPlatforms: Record<string, { dist: string; ext: "tar.xz" | "zip" }> = {
  "x86_64-pc-windows-msvc": { dist: "win64", ext: "zip" },
  "aarch64-pc-windows-msvc": { dist: "winarm64", ext: "zip" },
  "x86_64-unknown-linux-gnu": { dist: "linux64", ext: "tar.xz" },
  "aarch64-unknown-linux-gnu": { dist: "linuxarm64", ext: "tar.xz" },
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

async function checksumFrom(sumsUrl: string, file: string): Promise<string | undefined> {
  return new TextDecoder()
    .decode(await download(sumsUrl))
    .split("\n")
    .find((l) => l.trim().endsWith(` ${file}`) || l.trim().endsWith(`*${file}`))
    ?.split(/\s+/)[0];
}

async function extract(data: ArrayBuffer, archive: string, members: string[], outputs: string[]) {
  const work = await mkdtemp(join(tmpdir(), "stremio-sidecar-"));
  try {
    const archivePath = join(work, archive);
    await Bun.write(archivePath, data);
    await $`${tar} -xf ${archivePath} -C ${work} ${members}`;
    for (const [i, member] of members.entries()) {
      await copyFile(join(work, member), outputs[i]);
      await chmod(outputs[i], 0o755);
    }
  } finally {
    await rm(work, { recursive: true, force: true });
  }
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
  const [data, expected] = await Promise.all([download(`${base}/${archive}`), checksumFrom(`${base}/SHASUMS256.txt`, archive)]);
  if (!expected || expected !== sha256(data)) throw new Error(`Checksum mismatch for ${archive}`);
  await extract(data, archive, [exe ? `${name}/node.exe` : `${name}/bin/node`], [out]);
  console.log(`node: ${NODE_VERSION} -> ${out}`);
}

async function fetchServer() {
  const out = join(resDir, "server.js");
  if (existsSync(out) && !force) return console.log(`server.js: already present (${out})`);
  const url = (await Bun.file(serverUrlFile).text()).trim();
  console.log(`server.js: downloading ${url}`);
  await Bun.write(out, await download(url));
}

async function fetchFfmpegTools(triple: string, exe: string) {
  const outputs = ["ffmpeg", "ffprobe"].map((tool) => join(binDir, `${tool}-${triple}${exe}`));
  if (outputs.every((out) => existsSync(out)) && !force) return console.log("ffmpeg: already present");

  const platform = ffmpegPlatforms[triple];
  if (!platform) throw new Error(`No static ffmpeg build for target ${triple}; set FFMPEG_DIR instead`);
  const name = `ffmpeg-n${FFMPEG_BRANCH}-latest-${platform.dist}-gpl-${FFMPEG_BRANCH}`;
  const archive = `${name}.${platform.ext}`;

  console.log(`ffmpeg: downloading ${archive}`);
  const [data, expected] = await Promise.all([
    download(`${FFMPEG_RELEASES}/${archive}`),
    checksumFrom(`${FFMPEG_RELEASES}/checksums.sha256`, archive),
  ]);
  if (!expected || expected !== sha256(data)) throw new Error(`Checksum mismatch for ${archive}`);
  await extract(data, archive, ["ffmpeg", "ffprobe"].map((tool) => `${name}/bin/${tool}${exe}`), outputs);
  console.log(`ffmpeg: ${archive} -> ${binDir}`);
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
await (downloadFfmpeg ? fetchFfmpegTools(triple, exe) : copyFfmpegTools(triple, exe));
