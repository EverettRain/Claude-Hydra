// Downloads the pinned Mihomo kernel for the target platform into src-tauri/binaries/
// as a Tauri sidecar (mihomo-<target-triple>[.exe]). Every archive is checked against a
// pinned SHA-256 before use. Target: TAURI_ENV_TARGET_TRIPLE (set by `tauri build/dev`),
// else the first CLI argument, else the host triple from `rustc -vV`.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync, inflateRawSync } from "node:zlib";

const VERSION = "v1.19.33";
const ASSETS = {
  "aarch64-apple-darwin": ["darwin-arm64", "gz", "060b64b0f2c9a310831db0fe4adb2878de67bfa606c311813c1f13966aa504b6"],
  "x86_64-apple-darwin": ["darwin-amd64-compatible", "gz", "eefe967b74312f70b64efa0dfa54929d00c7af8e94416e1b12df09863a40f35b"],
  "x86_64-pc-windows-msvc": ["windows-amd64-compatible", "zip", "7aaf636a4f2172dcd8ec34ac4f52797b059195e5afd6409508c963e545f8864d"],
  "aarch64-pc-windows-msvc": ["windows-arm64", "zip", "a2a26869a14634c49f3fbcecc593b498ed8591fbdd7616b24475b3d60b148da3"],
};

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "binaries");
const hostTriple = () => /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
const triples = process.env.TAURI_ENV_TARGET_TRIPLE === "universal-apple-darwin"
  ? ["aarch64-apple-darwin", "x86_64-apple-darwin"]
  : [process.env.TAURI_ENV_TARGET_TRIPLE || process.argv[2] || hostTriple()];

/** Extracts the single file stored in a zip archive (stored or deflated). */
function unzipSingle(buf) {
  const eocd = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (eocd < 0) throw new Error("invalid zip");
  const cd = buf.readUInt32LE(eocd + 16);
  const method = buf.readUInt16LE(cd + 10);
  const compressed = buf.readUInt32LE(cd + 20);
  const size = buf.readUInt32LE(cd + 24);
  const local = buf.readUInt32LE(cd + 42);
  const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
  const data = buf.subarray(start, start + compressed);
  const out = method === 0 ? data : method === 8 ? inflateRawSync(data) : null;
  if (!out || out.length !== size) throw new Error("unsupported zip entry");
  return out;
}

mkdirSync(root, { recursive: true });
for (const triple of triples) {
  const asset = ASSETS[triple];
  if (!asset) throw new Error(`No bundled Mihomo for target ${triple}`);
  const [name, ext, sha256] = asset;
  const out = join(root, `mihomo-${triple}${triple.includes("windows") ? ".exe" : ""}`);
  const stamp = `${out}.version`;
  if (existsSync(out) && existsSync(stamp) && readFileSync(stamp, "utf8") === `${VERSION} ${sha256}`) continue;

  const url = `https://github.com/MetaCubeX/mihomo/releases/download/${VERSION}/mihomo-${name}-${VERSION}.${ext}`;
  console.log(`Downloading Mihomo ${VERSION} for ${triple}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Download failed (${res.status}): ${url}`);
  const archive = Buffer.from(await res.arrayBuffer());
  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== sha256) throw new Error(`Checksum mismatch for ${url}: ${actual}`);

  writeFileSync(out, ext === "gz" ? gunzipSync(archive) : unzipSingle(archive));
  chmodSync(out, 0o755);
  writeFileSync(stamp, `${VERSION} ${sha256}`);
}
