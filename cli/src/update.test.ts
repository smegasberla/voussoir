import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  CLI_VERSION,
  compareVersions,
  ensureTrustedUrl,
  githubFeed,
  installBytes,
  installTarget,
  platformKey,
  buildIsFresh,
  builtCommit,
  readChecksum,
  recordBuilt,
  sourceDir,
  stampPath,
  update,
} from "./commands/update.ts";
import { parseArgs } from "./args.ts";

describe("CLI version comparison", () => {
  test("compares numeric segments", () => {
    expect(compareVersions("1.10.0", "1.9.0")).toBe(1);
    expect(compareVersions("2.0.0", "2.0.0")).toBe(0);
    expect(compareVersions("1.0.0", "1.0.1")).toBe(-1);
    expect(compareVersions("1.0.0-beta.1", "1.0.0")).toBe(-1);
  });

  test("tracks package.json", () => {
    const pkg = JSON.parse(readFileSync(join(import.meta.dir, "..", "package.json"), "utf8"));
    expect(CLI_VERSION).toBe(pkg.version);
  });
});

describe("CLI platform keys", () => {
  test("uses the Rust updater's names", () => {
    expect(platformKey("linux", "x64")).toBe("linux-x86_64");
    expect(platformKey("win32", "x64")).toBe("windows-x86_64");
    expect(platformKey("darwin", "arm64")).toBe("macos-arm64");
  });
});

describe("CLI download trust", () => {
  test("https and loopback pass, anything else is refused", () => {
    expect(() => ensureTrustedUrl("https://example.com/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://127.0.0.1:9/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://localhost:9/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://example.com/app")).toThrow(/use https/);
    expect(() => ensureTrustedUrl("not a url")).toThrow(/not a URL/);
  });
});

describe("CLI install", () => {
  test("replaces the file atomically and leaves no temp behind", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      installBytes(Buffer.from("new"), target);
      expect(readFileSync(target, "utf8")).toBe("new");
      expect(statSync(target).mode & 0o111).not.toBe(0);
      expect(readdirSync(dir).filter((name) => name.includes(".tmp"))).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("voussoir update", () => {
  test("installs a newer, verified artifact", async () => {
    const artifact = Buffer.from("cli-binary-v2");
    const key = platformKey();
    let port = 0;
    const server = Bun.serve({
      port: 0,
      fetch(request: Request): Response {
        const path = new URL(request.url).pathname;
        if (path === "/feed") {
          return Response.json({
            version: "9.9.9",
            downloads: {
              [key]: {
                url: `http://127.0.0.1:${port}/app`,
                sha256: createHash("sha256").update(artifact).digest("hex"),
              },
            },
          });
        }
        if (path === "/app") return new Response(artifact);
        return new Response("not found", { status: 404 });
      },
    });
    port = server.port ?? 0;
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      const log = console.log;
      console.log = () => {};
      try {
        await update(
          parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--out", target]),
        );
      } finally {
        console.log = log;
      }
      expect(readFileSync(target)).toEqual(artifact);
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });

  test("a failed hash leaves the old CLI in place", async () => {
    const artifact = Buffer.from("cli-binary-v2");
    let port = 0;
    const server = Bun.serve({
      port: 0,
      fetch(request: Request): Response {
        const path = new URL(request.url).pathname;
        if (path === "/feed") {
          return Response.json({
            version: "9.9.9",
            url: `http://127.0.0.1:${port}/app`,
            sha256: "0".repeat(64),
          });
        }
        return new Response(artifact);
      },
    });
    port = server.port ?? 0;
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      await expect(
        update(parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--out", target])),
      ).rejects.toThrow(/does not match its sha256/);
      expect(readFileSync(target, "utf8")).toBe("old");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });

  test("--check does not download or replace", async () => {
    const server = Bun.serve({
      port: 0,
      fetch() {
        return Response.json({ version: "9.9.9", url: "http://127.0.0.1/x", sha256: "0".repeat(64) });
      },
    });
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      const logged: string[] = [];
      const log = console.log;
      console.log = (...parts: unknown[]) => logged.push(parts.join(" "));
      try {
        await update(
          parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--check", "--out", target]),
        );
      } finally {
        console.log = log;
      }
      expect(logged.join("\n")).toContain("update available");
      expect(readFileSync(target, "utf8")).toBe("old");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });
});

describe("GitHub release feeds", () => {
  const release = (assets: Array<{ name: string; browser_download_url: string }>) => ({
    tag_name: "v0.2.0",
    assets,
  });

  test("picks the asset for this platform and strips the v", () => {
    const feed = githubFeed(
      release([
        { name: "voussoir-cli-macos-arm64", browser_download_url: "https://example.com/mac" },
        { name: "voussoir-cli-linux-x86_64", browser_download_url: "https://example.com/linux" },
      ]),
      "linux-x86_64",
    );
    expect(feed.version).toBe("0.2.0");
    expect(feed.url).toBe("https://example.com/linux");
  });

  test("does not let one platform's key match another's", () => {
    const feed = githubFeed(
      release([
        { name: "voussoir-cli-linux-arm64", browser_download_url: "https://example.com/arm" },
        { name: "voussoir-cli-linux-x86_64", browser_download_url: "https://example.com/x86" },
      ]),
      "linux-x86_64",
    );
    expect(feed.url).toBe("https://example.com/x86");
  });

  test("carries a published checksum, and none when the release has none", () => {
    const withSums = githubFeed(
      release([
        { name: "voussoir-cli-linux-x86_64", browser_download_url: "https://example.com/x86" },
        { name: "sha256sums.txt", browser_download_url: "https://example.com/sums" },
      ]),
      "linux-x86_64",
    );
    expect(withSums.sha256).toBe("sha256sums.txt\u0000https://example.com/sums");

    const without = githubFeed(
      release([{ name: "voussoir-cli-linux-x86_64", browser_download_url: "https://example.com/x86" }]),
      "linux-x86_64",
    );
    expect(without.sha256).toBeUndefined();
  });

  test("refuses a release with no asset for the platform", () => {
    expect(() => githubFeed(release([]), "linux-x86_64")).toThrow(/no download for linux-x86_64/);
  });
});

describe("Checksum files", () => {
  test("reads the line naming the artifact out of a combined sums file", async () => {
    const body = [
      `${"a".repeat(64)}  voussoir-cli-macos-arm64`,
      `${"b".repeat(64)}  voussoir-cli-linux-x86_64`,
    ].join("\n");
    const server = Bun.serve({ port: 0, fetch: () => new Response(body) });
    try {
      const digest = await readChecksum(
        "sha256sums.txt",
        `http://127.0.0.1:${server.port}/sums`,
        "https://example.com/downloads/voussoir-cli-linux-x86_64",
      );
      expect(digest).toBe("b".repeat(64));
    } finally {
      server.stop(true);
    }
  });
});

describe("Update source resolution", () => {
  test("finds this checkout, so `update` has something to rebuild", () => {
    // Running the tests from a checkout is the case a developer hits, and the
    // old fixed guess at ~/.local/share/voussoir/src missed it entirely.
    const dir = sourceDir();
    expect(readdirSync(dir)).toContain("cli");
    expect(existsSync(join(dir, "cli", "package.json"))).toBe(true);
  });

  test("VOUSSOIR_SRC wins when it is set", () => {
    const previous = process.env.VOUSSOIR_SRC;
    process.env.VOUSSOIR_SRC = "/tmp/not-a-real-checkout";
    try {
      expect(sourceDir()).toBe("/tmp/not-a-real-checkout");
    } finally {
      if (previous === undefined) delete process.env.VOUSSOIR_SRC;
      else process.env.VOUSSOIR_SRC = previous;
    }
  });
});

describe("Update without a checkout", () => {
  test("says what is missing instead of guessing", async () => {
    const previous = process.env.VOUSSOIR_SRC;
    process.env.VOUSSOIR_SRC = join(tmpdir(), `voussoir-missing-${process.pid}`, "src");
    try {
      await expect(update(parseArgs(["update", "--check"]))).rejects.toThrow(/no source checkout/);
    } finally {
      if (previous === undefined) delete process.env.VOUSSOIR_SRC;
      else process.env.VOUSSOIR_SRC = previous;
    }
  });
});

describe("Update install target", () => {
  test("refuses to replace the runtime running a script", () => {
    // `bun cli/src/index.ts` reports bun itself as execPath. Installing there
    // would swap the toolchain for the CLI, which is unrecoverable.
    expect(installTarget("/home/me/.bun/bin/bun")).toBeUndefined();
    expect(installTarget("C:\\Users\\me\\.bun\\bun.exe")).toBeUndefined();
    expect(installTarget("/usr/bin/node")).toBeUndefined();
  });

  test("replaces an installed voussoir binary", () => {
    expect(installTarget("/home/me/.local/bin/voussoir")).toBe("/home/me/.local/bin/voussoir");
  });
});

describe("Build stamp", () => {
  test("records the commit the installed binary came from", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-stamp-"));
    const stamp = join(dir, "built");
    try {
      expect(builtCommit(stamp)).toBeUndefined();
      recordBuilt("abc1234", stamp);
      expect(builtCommit(stamp)).toBe("abc1234");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("an install with no stamp is treated as stale, not current", () => {
    // Every install made before the stamp existed lands here. Reporting it
    // "current" would be the original bug; one rebuild is the honest answer.
    const dir = mkdtempSync(join(tmpdir(), "voussoir-stamp-"));
    try {
      expect(builtCommit(join(dir, "absent"))).toBeUndefined();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("Freshness over cli/", () => {
  /** A throwaway repo with one commit, so `git diff` has real history to read. */
  function repo(): string {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-fresh-"));
    const run = (argv: string[]) => {
      const out = Bun.spawnSync(["git", "-C", dir, ...argv], {
        stdout: "pipe",
        stderr: "pipe",
      });
      if (out.exitCode !== 0) throw new Error(out.stderr.toString());
      return out.stdout.toString().trim();
    };
    run(["init", "-q"]);
    run(["config", "user.email", "t@example.invalid"]);
    run(["config", "user.name", "t"]);
    run(["config", "commit.gpgsign", "false"]);
    return dir;
  }

  const gitIn = (dir: string) => (argv: string[], allowFailure = false) => {
    const out = Bun.spawnSync(["git", "-C", dir, ...argv], { stdout: "pipe", stderr: "pipe" });
    if (out.exitCode !== 0 && !allowFailure) throw new Error(out.stderr.toString());
    return out.stdout.toString().trim();
  };

  test("a commit that does not touch cli/ keeps the binary current", () => {
    // The regression: comparing the binary's mtime to HEAD rebuilt the CLI for a
    // .gitignore change, because every commit moves HEAD past any binary.
    const dir = repo();
    try {
      writeFileSync(join(dir, ".gitignore"), "one\n");
      const git = gitIn(dir);
      git(["add", "."]);
      git(["commit", "-qm", "one"]);
      const first = git(["rev-parse", "HEAD"]);

      writeFileSync(join(dir, ".gitignore"), "two\n");
      git(["add", "."]);
      git(["commit", "-qm", "two"]);
      const second = git(["rev-parse", "HEAD"]);

      expect(buildIsFresh(git, dir, first, second)).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a commit that touches cli/ makes the binary stale", () => {
    const dir = repo();
    try {
      writeFileSync(join(dir, ".gitignore"), "x\n");
      const git = gitIn(dir);
      git(["add", "."]);
      git(["commit", "-qm", "one"]);
      const first = git(["rev-parse", "HEAD"]);

      mkdirSync(join(dir, "cli"), { recursive: true });
      writeFileSync(join(dir, "cli", "index.ts"), "export {};\n");
      git(["add", "."]);
      git(["commit", "-qm", "two"]);
      const second = git(["rev-parse", "HEAD"]);

      expect(buildIsFresh(git, dir, first, second)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a recorded commit this checkout does not have is stale", () => {
    // A rebase rewrites history, so a stamp can name a commit that is gone.
    // `git diff` then fails and prints nothing, which must not read as "fresh".
    const dir = repo();
    try {
      writeFileSync(join(dir, ".gitignore"), "x\n");
      const git = gitIn(dir);
      git(["add", "."]);
      git(["commit", "-qm", "one"]);
      const head = git(["rev-parse", "HEAD"]);

      expect(buildIsFresh(git, dir, "0".repeat(40), head)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("Direction against origin", () => {
  /** A checkout with a bare origin, so `rev-list` counts mean something. */
  function withOrigin(): { work: string; share: string; run: (argv: string[], cwd?: string) => string } {
    const root = mkdtempSync(join(tmpdir(), "voussoir-dir-"));
    const run = (argv: string[], cwd = root) => {
      const out = Bun.spawnSync(argv, { cwd, stdout: "pipe", stderr: "pipe" });
      if (out.exitCode !== 0) throw new Error(out.stderr.toString());
      return out.stdout.toString().trim();
    };
    run(["git", "init", "-q", "--bare", "origin.git"]);
    run(["git", "clone", "-q", "origin.git", "work"]);
    const work = join(root, "work");
    const identity: Array<[string, string]> = [
      ["user.email", "t@example.invalid"],
      ["user.name", "t"],
      ["commit.gpgsign", "false"],
    ];
    for (const [key, value] of identity) {
      run(["git", "config", key, value], work);
    }
    mkdirSync(join(work, "cli"), { recursive: true });
    writeFileSync(join(work, "cli", "package.json"), '{"name":"x"}\n');
    writeFileSync(join(work, "Cargo.toml"), '[package]\nname="x"\n');
    run(["git", "add", "."], work);
    run(["git", "commit", "-qm", "one"], work);
    run(["git", "push", "-q", "origin", "HEAD"], work);
    return { work, share: join(root, "share"), run };
  }

  const counts = (work: string, run: (a: string[], c?: string) => string) => {
    const branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"], work);
    return {
      behind: Number(run(["git", "rev-list", "--count", `HEAD..origin/${branch}`], work)),
      ahead: Number(run(["git", "rev-list", "--count", `origin/${branch}..HEAD`], work)),
    };
  };

  const outputOf = async (work: string, share: string): Promise<string> => {
    const previous = { src: process.env.VOUSSOIR_SRC, xdg: process.env.XDG_DATA_HOME };
    process.env.VOUSSOIR_SRC = work;
    process.env.XDG_DATA_HOME = share;
    mkdirSync(join(share, "voussoir"), { recursive: true });
    writeFileSync(
      join(share, "voussoir", "built"),
      Bun.spawnSync(["git", "-C", work, "rev-parse", "HEAD"]).stdout.toString(),
    );
    const logged: string[] = [];
    const log = console.log;
    console.log = (...parts: unknown[]) => logged.push(parts.join(" "));
    try {
      await update(parseArgs(["update", "--check"]));
    } finally {
      console.log = log;
      for (const [k, v] of [
        ["VOUSSOIR_SRC", previous.src],
        ["XDG_DATA_HOME", previous.xdg],
      ] as const) {
        if (v === undefined) delete process.env[k];
        else process.env[k] = v;
      }
    }
    return logged.join("\n");
  };

  test("a local commit ahead of origin is not an update", () => {
    // `before !== remote` called this behind and printed `af496eb -> 26abf61`,
    // a range that reads as an update but would move the binary backwards.
    const { work, share, run } = withOrigin();
    try {
      writeFileSync(join(work, "local.txt"), "x\n");
      run(["git", "add", "."], work);
      run(["git", "commit", "-qm", "local only"], work);
      expect(counts(work, run)).toEqual({ behind: 0, ahead: 1 });
    } finally {
      rmSync(join(work, ".."), { recursive: true, force: true });
    }
  });

  test("says current and names the unpushed commit", async () => {
    const { work, share, run } = withOrigin();
    try {
      writeFileSync(join(work, "local.txt"), "x\n");
      run(["git", "add", "."], work);
      run(["git", "commit", "-qm", "local only"], work);
      const output = await outputOf(work, share);
      expect(output).toContain("is current");
      expect(output).toContain("1 local commit not pushed");
      expect(output).not.toContain("update available");
    } finally {
      rmSync(join(work, ".."), { recursive: true, force: true });
    }
  });

  test("a genuinely older checkout still reports an update", async () => {
    const { work, share, run } = withOrigin();
    const root = join(work, "..");
    try {
      // Someone else pushes; this checkout has not seen it.
      run(["git", "clone", "-q", "origin.git", "other"], root);
      const other = join(root, "other");
      run(["git", "config", "user.email", "t@example.invalid"], other);
      run(["git", "config", "user.name", "t"], other);
      writeFileSync(join(other, "new.txt"), "x\n");
      run(["git", "add", "."], other);
      run(["git", "commit", "-qm", "later"], other);
      run(["git", "push", "-q", "origin", "HEAD"], other);

      const output = await outputOf(work, share);
      expect(output).toContain("update available");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
