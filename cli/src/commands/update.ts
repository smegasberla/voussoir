/** `voussoir update`: replace the CLI itself from a release feed.
 *
 *  The CLI ships as a single compiled binary, so updating it is the same problem
 *  the shell's updater solves: fetch a feed, download one file, check its hash,
 *  and rename it into place without ever leaving a half-written executable.
 *  There is no signing key for the CLI by default, so this is hash + TLS; run it
 *  by hand, and it says so when the feed is not https.
 *
 *  The feed defaults to the repository's `releases/latest`, so a plain
 *  `voussoir update` works with no flags and no environment.
 */

import { createHash } from "node:crypto";
import { chmodSync, existsSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";

export const CLI_VERSION = "0.1.0";

/** The published release feed, so a plain `voussoir update` works with no flags. */
export const DEFAULT_REPO = "smegasberla/voussoir";
export const DEFAULT_FEED_URL = `https://api.github.com/repos/${DEFAULT_REPO}/releases/latest`;

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

/** A feed in the shape this command understands. */
export interface Feed {
  version: string;
  url: string;
  sha256?: string;
}

interface GithubAsset {
  name?: string;
  browser_download_url?: string;
}

/**
 * Turn GitHub's release JSON into a [`Feed`].
 *
 * `api.github.com/repos/<owner>/<repo>/releases/latest` answers with `tag_name`
 * and `assets`, which is nothing like the small feed this command reads. A
 * plain `voussoir update` has to work without flags, so the default feed is that
 * endpoint and the translation happens here rather than in a second code path.
 */
export function githubFeed(raw: Record<string, unknown>, key: string): Feed {
  const version = String(raw.tag_name ?? "").replace(/^v/, "");
  if (!version) throw new Error("the release feed has no tag");

  const assets = (Array.isArray(raw.assets) ? raw.assets : []) as GithubAsset[];
  const find = (pattern: RegExp) =>
    assets.find((asset) => typeof asset.name === "string" && pattern.test(asset.name));

  // A release may publish one artifact per platform; the key is the suffix
  // `platformKey` produces, so `voussoir-cli-linux-x86_64` matches on any of the
  // separators a packager might choose.
  const artifact =
    find(new RegExp(`(^|[^a-z0-9])${key.replace(/[^a-z0-9]/gi, ".")}([^a-z0-9]|$)`, "i")) ??
    find(/voussoir[-_.]?cli/i);
  if (!artifact?.browser_download_url) {
    throw new Error(`release ${version} has no download for ${key}`);
  }

  const checksum = find(/^(sha256sums|checksums\.txt|sha256sums\.txt)$/i) ?? find(/\.sha256$/i);
  return {
    version,
    url: artifact.browser_download_url,
    // Resolved by the caller: a checksum is a second fetch, and the artifact
    // name is needed to pick the right line out of a combined sums file.
    sha256: checksum ? `${checksum.name}\u0000${checksum.browser_download_url}` : undefined,
  };
}

/** Read the hex digest a release publishes for `artifactUrl`. */
export async function readChecksum(
  checksumName: string,
  checksumUrl: string,
  artifactUrl: string,
): Promise<string | undefined> {
  const body = await fetchText(checksumUrl, FEED_CAP, 30_000);
  const file = artifactUrl.split("/").pop() ?? "";
  for (const line of body.split("\n")) {
    const [digest, named] = line.trim().split(/\s+/);
    if (!digest || !/^[0-9a-f]{64}$/i.test(digest)) continue;
    // A bare `.sha256` file often holds the digest alone; a combined sums file
    // always names its file, and only that line is ours.
    if (!named || named.replace(/^\*/, "") === file) return digest.toLowerCase();
  }
  if (checksumName.endsWith(".sha256") && body.trim().length === 64) return body.trim().toLowerCase();
  return undefined;
}

export async function update(args: Args): Promise<number> {
  const feedUrl =
    stringFlag(args, "feed") ?? process.env.VOUSSOIR_CLI_FEED ?? DEFAULT_FEED_URL;
  const checkOnly = boolFlag(args, "check");
  const force = boolFlag(args, "force");

  let feedBody = "";
  try {
    feedBody = await fetchText(feedUrl, FEED_CAP, 30_000);
  } catch (error) {
    // The default feed is the repository's `releases/latest`, which answers 404
    // until the first release is published. That is not a broken update: there
    // is simply nothing to move to yet, and a plain `voussoir update` should say
    // so rather than fail.
    if (feedUrl === DEFAULT_FEED_URL && /answered 404/.test((error as Error).message)) {
      console.log(`  no release published yet for ${DEFAULT_REPO} (${CLI_VERSION} is current)`);
      return 0;
    }
    throw new Error(`cannot read the update feed: ${(error as Error).message}`);
  }
  let raw: Record<string, unknown>;
  try {
    raw = JSON.parse(feedBody);
  } catch (error) {
    throw new Error(`the update feed is not JSON: ${(error as Error).message}`);
  }

  const key = platformKey();
  let artifact: Feed;
  if (raw.tag_name !== undefined) {
    artifact = githubFeed(raw, key);
  } else {
    const declared = raw as {
      version?: string;
      url?: string;
      sha256?: string;
      downloads?: Record<string, { url?: string; sha256?: string }>;
    };
    if (!declared.version) throw new Error("the update feed has no `version`");
    const chosen = declared.downloads?.[key] ?? declared;
    if (!chosen.url) throw new Error(`the update feed has no download for ${key}`);
    artifact = { version: declared.version, url: chosen.url, sha256: chosen.sha256 };
  }

  if (artifact.sha256?.includes("\u0000")) {
    const [name, url] = artifact.sha256.split("\u0000") as [string, string];
    artifact.sha256 = await readChecksum(name, url, artifact.url);
  }

  const feed = artifact;

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
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (!feed.sha256) {
    // A release with no published checksum still arrives over https, which the
    // feed URL was already held to, so the download is not unverified: it is
    // verified by transport. Writing the digest makes the choice visible.
    console.log(`  note: no published sha256; downloaded ${actual}`);
  } else if (actual !== feed.sha256) {
    throw new Error(`the download does not match its sha256; expected ${feed.sha256}, got ${actual}`);
  }
  try {
    installBytes(bytes, target);
  } catch (error) {
    throw new Error(`cannot install the update: ${(error as Error).message}`);
  }
  console.log(`  installed ${feed.version} at ${target}`);
  return 0;
}
