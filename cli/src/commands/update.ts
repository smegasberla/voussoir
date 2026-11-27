/** `voussoir update`: replace the CLI itself from a release feed.
 *
 *  The CLI ships as a single compiled binary, so updating it is the same problem
 *  the shell's updater solves: fetch a feed, download one file, check its hash,
 *  and rename it into place without ever leaving a half-written executable.
 *  There is no signing key for the CLI by default, so this is hash + TLS; run it
 *  by hand, and it says so when the feed is not https.
 *
 *  A plain `voussoir update` rebuilds from the checkout `install.sh` made, which
 *  is how this project is distributed: there are no published releases to read.
 *  `--feed <url>` is still accepted for anyone shipping prebuilt binaries.
 */

import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";

export const CLI_VERSION = "0.1.0";


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

/** A directory that looks like this project's checkout. */
function isCheckout(dir: string): boolean {
  return existsSync(join(dir, "cli", "package.json")) && existsSync(join(dir, "Cargo.toml"));
}

/**
 * The directory the installers record the checkout and built commit in.
 *
 * `install.ps1` uses `%LOCALAPPDATA%\voussoir`, the shell scripts use
 * `~/.local/share/voussoir`. Resolving it in one place keeps `update` from
 * looking in a directory its own installer never wrote, which on Windows meant
 * every install read as stale and every `update` failed to find a checkout.
 */
export function shareDir(
  env: NodeJS.ProcessEnv = process.env,
  platform: string = process.platform,
  home: string = homedir(),
): string {
  if (env.XDG_DATA_HOME) return env.XDG_DATA_HOME;
  if (platform === "win32" && env.LOCALAPPDATA) return env.LOCALAPPDATA;
  return join(home, ".local", "share");
}

/**
 * Where `install.sh` keeps the checkout it builds the CLI from.
 *
 * Three answers, in order of how directly they say it: the environment, the
 * pointer `install.sh` now writes beside the installed binary, and walking up
 * from this source file, which is what running `bun cli/src/index.ts` uses. The
 * old single guess at `~/.local/share/voussoir/src` only held when the installer
 * had cloned, so a developer's own checkout was invisible to it.
 */
export function sourceDir(): string {
  if (process.env.VOUSSOIR_SRC) return process.env.VOUSSOIR_SRC;
  const share = shareDir();

  const pointer = join(share, "voussoir", "source");
  if (existsSync(pointer)) {
    const recorded = readFileSync(pointer, "utf8").trim();
    if (recorded && isCheckout(recorded)) return recorded;
  }

  let dir = import.meta.dir;
  for (;;) {
    if (isCheckout(dir)) return dir;
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  return join(share, "voussoir", "src");
}

export async function update(args: Args): Promise<number> {
  const checkOnly = boolFlag(args, "check");
  const force = boolFlag(args, "force");

  // Nothing is published as a release, so the honest default is the same source
  // tree `install.sh` builds from: fetch, rebuild, replace. `--feed` stays for
  // anyone serving prebuilt binaries of their own.
  const feedUrl = stringFlag(args, "feed") ?? process.env.VOUSSOIR_CLI_FEED;
  if (feedUrl) return await updateFromFeed(feedUrl, args, checkOnly, force);
  return await updateFromSource(checkOnly, force);
}

/**
 * Where the commit the installed binary was built from is recorded.
 *
 * Beside the `source` pointer `install.sh` writes, because the two answer
 * halves of the same question: which checkout, and which commit of it.
 */
export function stampPath(): string {
  return join(shareDir(), "voussoir", "built");
}

/** The commit recorded for this install, or `undefined` when there is none. */
export function builtCommit(stamp: string = stampPath()): string | undefined {
  if (!existsSync(stamp)) return undefined;
  const recorded = readFileSync(stamp, "utf8").trim();
  return recorded === "" ? undefined : recorded;
}

/** Record `commit` as the one the installed binary was built from. */
export function recordBuilt(commit: string, stamp: string = stampPath()): void {
  mkdirSync(dirname(stamp), { recursive: true });
  writeFileSync(stamp, `${commit}\n`);
}

/**
 * True when a binary built from `recorded` is still current for `head`.
 *
 * An unknown stamp means an install made before the stamp existed, which is one
 * rebuild away from knowing. A commit unreachable from this checkout (after a
 * rebase) is treated the same way, because it cannot be diffed either.
 */
export function buildIsFresh(
  git: (argv: string[], allowFailure?: boolean) => string,
  dir: string,
  recorded: string | undefined,
  head: string,
): boolean {
  if (recorded === undefined) return false;
  if (recorded === head) return true;
  // A commit this checkout does not have (after a rebase, or a stamp copied from
  // another machine) cannot be diffed: `git diff` exits non-zero and prints
  // nothing, which would otherwise read as "no changes since".
  const known = Bun.spawnSync(["git", "-C", dir, "cat-file", "-e", `${recorded}^{commit}`], {
    stdout: "pipe",
    stderr: "pipe",
  });
  if (known.exitCode !== 0) return false;
  const changed = git(["diff", "--name-only", `${recorded}..${head}`, "--", "cli"], true);
  return changed === "";
}

async function updateFromSource(checkOnly: boolean, force: boolean): Promise<number> {
  const dir = sourceDir();
  if (!existsSync(join(dir, ".git"))) {
    throw new Error(
      `no source checkout at ${dir}; run install.sh, or pass --feed <url> for a prebuilt binary`,
    );
  }

  const git = (argv: string[], allowFailure = false): string => {
    const out = Bun.spawnSync(["git", "-C", dir, ...argv], { stdout: "pipe", stderr: "pipe" });
    if (out.exitCode !== 0 && !allowFailure) {
      throw new Error(`git ${argv.join(" ")} failed: ${out.stderr.toString().trim()}`);
    }
    return out.stdout.toString().trim();
  };

  const before = git(["rev-parse", "HEAD"]);
  git(["fetch", "--quiet", "origin"]);
  const branch = git(["rev-parse", "--abbrev-ref", "HEAD"]);
  const remote = git(["rev-parse", `origin/${branch}`]);

  // Directional, not `before !== remote`. A branch with local commits differs
  // from origin in the other direction, and calling that "behind" printed
  // `af496eb -> 26abf61`, which reads as an update but would go backwards.
  const behindCount = Number(git(["rev-list", "--count", `HEAD..origin/${branch}`]));
  const aheadCount = Number(git(["rev-list", "--count", `origin/${branch}..HEAD`]));
  const behind = behindCount > 0;

  // The checkout matching origin says nothing about the installed binary, which
  // is the thing you actually run. Compare the commit it was built from against
  // HEAD instead, and only over `cli/`: an mtime comparison forced a full
  // rebuild for a commit that touched .gitignore, which is how this was found.
  const target = installTarget();
  const recorded = builtCommit();
  const stale = target !== undefined && !buildIsFresh(git, dir, recorded, before);

  if (!behind && !stale && !force) {
    // Unpushed local work is worth saying out loud: the binary is current for
    // this checkout even though origin does not have those commits yet.
    const local =
      aheadCount === 1 ? "1 local commit not pushed" : `${aheadCount} local commits not pushed`;
    const suffix = aheadCount > 0 ? `, ${local}` : "";
    console.log(`  ${CLI_VERSION} is current (checkout at ${before.slice(0, 7)}${suffix})`);
    return 0;
  }
  if (!behind && stale) {
    console.log(
      `  installed binary was built from ${recorded?.slice(0, 7) ?? "an unknown commit"}; rebuilding`,
    );
  }
  if (checkOnly) {
    // Printing "50334d7 -> 50334d7" when only the binary is stale reads as a
    // bug, so say which half is behind.
    console.log(
      behind
        ? `  update available: ${before.slice(0, 7)} -> ${remote.slice(0, 7)} in ${dir}`
        : `  update available: the installed binary is behind ${before.slice(0, 7)}`,
    );
    return 0;
  }

  // Fast-forward only: a checkout with local commits is someone's work in
  // progress, and replacing their binary over it would be the wrong move. When
  // the branch has diverged there is no fast-forward to make, so say that rather
  // than letting git fail with "Not possible to fast-forward".
  if (behind && aheadCount > 0) {
    throw new Error(
      `${aheadCount} local commit(s) on ${branch} block the update; push or stash them, then run update again`,
    );
  }
  git(["merge", "--ff-only", `origin/${branch}`]);

  console.log("  rebuilding...");
  const build = join(dir, "cli");
  for (const argv of [["bun", "install"], ["bun", "run", "build"]]) {
    const out = Bun.spawnSync(argv, { cwd: build, stdout: "pipe", stderr: "pipe" });
    if (out.exitCode !== 0) {
      throw new Error(`${argv.join(" ")} failed: ${out.stderr.toString().trim()}`);
    }
  }

  const built = join(build, "dist", "voussoir");
  if (!existsSync(built)) throw new Error(`the build finished but ${built} is missing`);

  // Only a compiled binary can be replaced. Run as `bun cli/src/index.ts`,
  // `process.execPath` is the Bun runtime itself, and installing over it would
  // destroy the toolchain the rebuild just used.
  if (target === undefined) {
    console.log(`  rebuilt ${remote.slice(0, 7)}; ${built} is ready but nothing is installed to replace`);
    console.log("  run install.sh to install the CLI");
    return 0;
  }
  try {
    installBytes(readFileSync(built), target);
  } catch (error) {
    throw new Error(`cannot replace ${target}: ${(error as Error).message}`);
  }
  // Only after the binary is in place: a stamp written before the copy would
  // describe a build that never landed, and hide the next update.
  recordBuilt(git(["rev-parse", "HEAD"]));
  console.log(`  rebuilt ${remote.slice(0, 7)} and replaced ${target}`);
  return 0;
}

/**
 * The installed CLI to replace, or `undefined` when running from source.
 *
 * A compiled binary's `execPath` is itself; a script's is the runtime running
 * it. Writing the new binary over the runtime would replace Bun with the CLI.
 */
export function installTarget(execPath: string = process.execPath): string | undefined {
  // Both separators: `path.basename` is platform-specific, and a Windows path
  // handed to it on Unix comes back whole, which would defeat the check.
  const base = (execPath.split(/[\\/]/).pop() ?? "").toLowerCase();
  if (base === "bun" || base === "bun.exe" || base === "node" || base === "node.exe") {
    return undefined;
  }
  return execPath;
}

async function updateFromFeed(
  feedUrl: string,
  args: Args,
  checkOnly: boolean,
  force: boolean,
): Promise<number> {
  let feedBody = "";
  try {
    feedBody = await fetchText(feedUrl, FEED_CAP, 30_000);
  } catch (error) {
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

  // `--out` names where to write, so it is honoured as given. The fallback to
  // the running executable goes through the same guard as a rebuild: run from a
  // script that is Bun itself, and installing there would replace the runtime.
  const target = stringFlag(args, "out") ?? installTarget();
  if (target === undefined) {
    throw new Error(
      `download ready, but nothing is installed to replace; pass --out <path> or run install.sh`,
    );
  }
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
