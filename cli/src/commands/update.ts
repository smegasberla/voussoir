/** `voussoir update`: replace the CLI itself from a release feed.
 *
 *  The CLI ships as a single compiled binary, so updating it is the same problem
 *  the shell's updater solves: fetch a feed, download one file, check its hash,
 *  and rename it into place without ever leaving a half-written executable.
 *  There is no signing key for the CLI by default, so this is hash + TLS; run it
 *  by hand, and it says so when the feed is not https.
 */

import { createHash } from "node:crypto";
import { chmodSync, existsSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";

export const CLI_VERSION = "0.2.0";

const FEED_CAP = 64 * 1024;
const ARTIFACT_CAP = 256 * 1024 * 1024;

/** The key a feed uses for this build, matching the Rust updater's names. */
export function platformKey(
  platform: string = process.platform,
  arch: string = process.arch,
): string {
  const os = platform === "win32" ? "windows" : platform === "darwin" ? "macos" : platform;
  const cpu = arch === "x64" ? "x86_64" : arch === "ia32" ? "x86" : arch;
  return `${os}-${cpu}`;
}

/** Numeric segment comparison; `1.10.0` is newer than `1.9.0`. */
export function compareVersions(a: string, b: string): number {
  const parse = (value: string) => {
    const withoutBuild = value.split("+")[0]!;
    const dash = withoutBuild.indexOf("-");
    const core = dash === -1 ? withoutBuild : withoutBuild.slice(0, dash);
    const pre = dash === -1 ? "" : withoutBuild.slice(dash + 1);
    return { core: core.split(".").filter(Boolean), pre: pre.split(".").filter(Boolean) };
  };
  const left = parse(a);
  const right = parse(b);
  const length = Math.max(left.core.length, right.core.length);
  for (let i = 0; i < length; i++) {
    const l = Number(left.core[i] ?? "0");
    const r = Number(right.core[i] ?? "0");
    if (l !== r) return l < r ? -1 : 1;
  }
  // No prerelease means the final release, which is newer than any prerelease.
  if (left.pre.length === 0 && right.pre.length > 0) return 1;
  if (right.pre.length === 0 && left.pre.length > 0) return -1;
  const preLength = Math.max(left.pre.length, right.pre.length);
  for (let i = 0; i < preLength; i++) {
    const l = left.pre[i];
    const r = right.pre[i];
    if (l === undefined) return -1;
    if (r === undefined) return 1;
    const ln = Number(l);
    const rn = Number(r);
    if (Number.isFinite(ln) && Number.isFinite(rn)) {
      if (ln !== rn) return ln < rn ? -1 : 1;
    } else if (ln !== rn) {
      if (Number.isFinite(ln)) return -1;
      if (Number.isFinite(rn)) return 1;
      if (l !== r) return l < r ? -1 : 1;
    }
  }
  return 0;
}

async function fetchText(url: string, cap: number, timeoutMs: number): Promise<string> {
  ensureTrustedUrl(url);
  const response = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  const declared = Number(response.headers.get("content-length") ?? "NaN");
  if (Number.isFinite(declared) && declared > cap) {
    throw new Error(`${url} is larger than ${cap} bytes`);
  }
  const text = await response.text();
  if (text.length > cap) throw new Error(`${url} is larger than ${cap} bytes`);
  return text;
}

async function fetchBytes(url: string, cap: number, timeoutMs: number): Promise<Buffer> {
  ensureTrustedUrl(url);
  const response = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  const declared = Number(response.headers.get("content-length") ?? "NaN");
  if (Number.isFinite(declared) && declared > cap) {
    throw new Error(`${url} is larger than ${cap} bytes`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > cap) throw new Error(`${url} is larger than ${cap} bytes`);
  return bytes;
}

/** `https`, or loopback for a test server; anything else can be rewritten. */
export function ensureTrustedUrl(url: string): void {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error(`${url} is not a URL`);
  }
  if (parsed.protocol === "https:") return;
  const loopback =
    parsed.hostname === "localhost" ||
    parsed.hostname === "127.0.0.1" ||
    parsed.hostname === "[::1]" ||
    parsed.hostname === "::1";
  if (parsed.protocol === "http:" && loopback) return;
  throw new Error(`refusing ${parsed.protocol}// to ${parsed.host}: use https`);
}

/** Install `bytes` at `target`, using the same rename dance as the shell. */
export function installBytes(bytes: Buffer, target: string): void {
  const temp = join(dirname(target), `.${process.pid}-voussoir-update.tmp`);
  writeFileSync(temp, bytes, { mode: 0o755 });
  chmodSync(temp, 0o755);
  const backup = `${target}.old`;
  if (process.platform === "win32") {
    let steppedAside = false;
    if (existsSync(target)) {
      renameSync(target, backup);
      steppedAside = true;
    }
    try {
      renameSync(temp, target);
    } catch (error) {
      if (steppedAside) renameSync(backup, target);
      rmSync(temp, { force: true });
      throw error;
    }
    // The running executable is still mapped by this process, so removing the
    // backup can fail; the next install cleans it up.
    try {
      rmSync(backup, { force: true });
    } catch {
      // locked until this process exits
    }
  } else {
    renameSync(temp, target);
  }
}

export async function update(args: Args): Promise<number> {
  const feedUrl = stringFlag(args, "feed") ?? process.env.VOUSSOIR_CLI_FEED;
  if (!feedUrl) {
    throw new Error("update needs --feed <url> (or VOUSSOIR_CLI_FEED): the CLI has no default release server");
  }
  const checkOnly = boolFlag(args, "check");
  const force = boolFlag(args, "force");

  let feedBody = "";
  try {
    feedBody = await fetchText(feedUrl, FEED_CAP, 30_000);
  } catch (error) {
    throw new Error(`cannot read the update feed: ${(error as Error).message}`);
  }
  let feed: {
    version?: string;
    url?: string;
    sha256?: string;
    downloads?: Record<string, { url?: string; sha256?: string }>;
  };
  try {
    feed = JSON.parse(feedBody);
  } catch (error) {
    throw new Error(`the update feed is not JSON: ${(error as Error).message}`);
  }
  if (!feed.version) throw new Error("the update feed has no `version`");

  const key = platformKey();
  const artifact = feed.downloads?.[key] ?? feed;
  if (!artifact.url) throw new Error(`the update feed has no download for ${key}`);
  if (!artifact.sha256) throw new Error("the update feed has no `sha256`; refusing an unverified download");

  const comparison = compareVersions(feed.version, CLI_VERSION);
  if (comparison <= 0 && !force) {
    console.log(`  ${CLI_VERSION} is current (feed says ${feed.version})`);
    return 0;
  }
  if (checkOnly) {
    console.log(`  update available: ${CLI_VERSION} -> ${feed.version}`);
    return 0;
  }

  const target = stringFlag(args, "out") ?? process.execPath;
  console.log(`  downloading ${feed.version} for ${key}...`);
  let bytes: Buffer;
  try {
    bytes = await fetchBytes(artifact.url, ARTIFACT_CAP, 10 * 60 * 1000);
  } catch (error) {
    throw new Error(`cannot download the update: ${(error as Error).message}`);
  }
  const expectedSha = artifact.sha256!;
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expectedSha.toLowerCase()) {
    throw new Error(`the download does not match its sha256; expected ${expectedSha}, got ${actual}`);
  }
  try {
    installBytes(bytes, target);
  } catch (error) {
    throw new Error(`cannot install the update: ${(error as Error).message}`);
  }
  console.log(`  installed ${feed.version} at ${target}`);
  return 0;
}
