#!/usr/bin/env node
// Stages FFmpeg/FFprobe sidecars into src-tauri/binaries/ with the target-triple
// names Tauri expects.
//
//   node scripts/fetch-ffmpeg.mjs --target <triple>   download, verify SHA-256, extract
//   node scripts/fetch-ffmpeg.mjs --local             copy this machine's ffmpeg (dev only, never ship)
//
// Fails closed: a release build never runs with an unverified or empty checksum.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "binaries");
const manifest = JSON.parse(readFileSync(join(root, "scripts", "ffmpeg-manifest.json"), "utf8"));
const args = process.argv.slice(2);

const hostTriple = () => {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  return /host: (\S+)/.exec(out)[1];
};

const exeSuffix = (triple) => (triple.includes("windows") ? ".exe" : "");

function findFile(dir, name) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) {
      const hit = findFile(p, name);
      if (hit) return hit;
    } else if (entry === name) {
      return p;
    }
  }
  return null;
}

async function fetchTarget(triple) {
  const spec = manifest.targets[triple];
  if (!spec) throw new Error(`no manifest entry for ${triple}`);
  if (!spec.url || !spec.sha256) {
    throw new Error(`manifest entry for ${triple} has no url/sha256; pin a verified LGPL build first`);
  }
  const res = await fetch(spec.url);
  if (!res.ok) throw new Error(`download failed: ${res.status} ${spec.url}`);
  const bytes = Buffer.from(await res.arrayBuffer());
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== spec.sha256.toLowerCase()) {
    throw new Error(`checksum mismatch for ${spec.url}: got ${digest}`);
  }

  const work = mkdtempSync(join(tmpdir(), "ffmpeg-"));
  const archive = join(work, spec.url.split("/").pop());
  writeFileSync(archive, bytes);
  // bsdtar/GNU tar both read .zip and .tar.* on the platforms we build for.
  execFileSync("tar", ["-xf", archive, "-C", work]);

  mkdirSync(outDir, { recursive: true });
  for (const [kind, name] of [["ffmpeg", spec.ffmpeg], ["ffprobe", spec.ffprobe]]) {
    const src = findFile(work, name);
    if (!src) throw new Error(`${name} not found in archive`);
    const dest = join(outDir, `${kind}-${triple}${exeSuffix(triple)}`);
    copyFileSync(src, dest);
    chmodSync(dest, 0o755);
    console.log(`staged ${dest}`);
  }
}

function copyLocal(triple) {
  mkdirSync(outDir, { recursive: true });
  for (const name of ["ffmpeg", "ffprobe"]) {
    // `where` is the Windows equivalent of `which`; it can print several matches, so take the first.
    const lookup = process.platform === "win32" ? "where" : "which";
    const found = execFileSync(lookup, [name], { encoding: "utf8" }).split(/\r?\n/)[0].trim();
    if (!found || !existsSync(found)) throw new Error(`${name} not on PATH`);
    const dest = join(outDir, `${name}-${triple}${exeSuffix(triple)}`);
    copyFileSync(found, dest);
    chmodSync(dest, 0o755);
    console.log(`copied local ${found} -> ${dest} (dev only)`);
  }
}

const triple = args.includes("--target") ? args[args.indexOf("--target") + 1] : hostTriple();
try {
  if (args.includes("--local")) copyLocal(triple);
  else await fetchTarget(triple);
} catch (err) {
  console.error(`fetch-ffmpeg: ${err.message}`);
  process.exit(1);
}
