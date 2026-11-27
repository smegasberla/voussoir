import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Args } from "./args.ts";
import { init } from "./commands/init.ts";

/** `init` prints a file per line; the tests care about the tree, not the log. */
const quiet = <T>(run: () => T): T => {
  const log = spyOn(console, "log").mockImplementation(() => {});
  try {
    return run();
  } finally {
    log.mockRestore();
  }
};

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function scaffold(flags: Args["flags"] = {}): string {
  const root = mkdtempSync(join(tmpdir(), "voussoir-init-"));
  roots.push(root);
  const dir = join(root, "app");
  const args: Args = { command: "init", positionals: [dir], flags, help: false };
  expect(quiet(() => init(args))).toBe(0);
  return dir;
}

/** Every file in a scaffold, relative to it, so content checks can walk the tree. */
function files(dir: string, prefix = ""): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(join(dir, prefix), { withFileTypes: true })) {
    const rel = prefix === "" ? entry.name : join(prefix, entry.name);
    if (entry.isDirectory()) found.push(...files(dir, rel));
    else found.push(rel);
  }
  return found;
}

/** Like `scaffold`, but keeps what `init` printed, so the hints can be checked. */
function scaffoldOutput(flags: Args["flags"]): string {
  const root = mkdtempSync(join(tmpdir(), "voussoir-init-"));
  roots.push(root);
  const dir = join(root, "app");
  const lines: string[] = [];
  const log = spyOn(console, "log").mockImplementation((...parts: unknown[]) => {
    lines.push(parts.join(" "));
  });
  try {
    expect(init({ command: "init", positionals: [dir], flags, help: false })).toBe(0);
  } finally {
    log.mockRestore();
  }
  return lines.join("\n");
}

describe("voussoir init", () => {
  test("a default project has both shells", () => {
    const dir = scaffold({ name: "Both Shells" });
    expect(existsSync(join(dir, "native", "src", "main.rs"))).toBe(true);
    expect(existsSync(join(dir, "android", "src", "lib.rs"))).toBe(true);
  });

  test("--mobile writes no desktop crate", () => {
    // The point of the flag: an app that only ships to Play should not carry a
    // `native/` tree it never builds. A stale one is worse than none, because
    // `checkParity` then compares against a command list nobody maintains.
    const dir = scaffold({ name: "Mobile Only", mobile: true });
    expect(existsSync(join(dir, "native"))).toBe(false);
    expect(existsSync(join(dir, "android", "src", "lib.rs"))).toBe(true);
    expect(existsSync(join(dir, "index.html"))).toBe(true);
  });

  test("--mobile still writes every Android file a build needs", () => {
    const dir = scaffold({ name: "Mobile Only", mobile: true });
    const java = join(dir, "android", "app", "src", "main", "java", "com");
    expect(existsSync(join(dir, "android", "app", "build.gradle"))).toBe(true);
    expect(existsSync(join(dir, "android", "app", "src", "main", "AndroidManifest.xml"))).toBe(true);

    // `android build` compiles Java only after wry generates `MainActivity`, and
    // the shell is what the manifest names, so a missing activity is a broken
    // scaffold that still reports success.
    const activity = readdirSync(java, { recursive: true }).some((entry) =>
      String(entry).endsWith("MainActivity.java"),
    );
    expect(activity).toBe(true);
  });

  test("a non-empty directory is refused without --force", () => {
    const dir = scaffold({ name: "Once" });
    const args: Args = { command: "init", positionals: [dir], flags: {}, help: false };
    const error = spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(init(args)).toBe(1);
    } finally {
      error.mockRestore();
    }
  });

  test("--force rewrites over a non-empty directory", () => {
    const dir = scaffold({ name: "Once" });
    const args: Args = { command: "init", positionals: [dir], flags: { force: true }, help: false };
    expect(quiet(() => init(args))).toBe(0);
  });

  test("the scaffolded crate names the app it belongs to", () => {
    const dir = scaffold({ name: "Mobile Only", mobile: true });
    const cargo = readFileSync(join(dir, "android", "Cargo.toml"), "utf8");
    expect(cargo).toContain("mobile_only");
    const manifest = readFileSync(join(dir, "android", "app", "src", "main", "AndroidManifest.xml"), "utf8");
    expect(manifest).not.toContain("{{");
  });

  test("the config names the app and ships no placeholder updater", () => {
    // The scaffold used to write `"update": { "url": "https://example.com/..." }`,
    // so every new app shipped an updater pointed at a host that does not serve
    // it: the first check failed and the app reported a broken update to someone
    // who had not asked for one. `update` is optional in the shell's config, so
    // the honest default is to leave it out and let the user add a real endpoint.
    const dir = scaffold({ name: "Config Check" });
    const config = JSON.parse(readFileSync(join(dir, "voussoir.json"), "utf8"));
    expect(config.name).toBe("Config Check");
    expect(config.width).toBe(900);
    expect(config.update).toBeUndefined();
  });

  test("no scaffolded file is left with an unfilled placeholder", () => {
    // A template that references a key `init` does not provide silently emits
    // `{{SOMETHING}}`, which then fails somewhere far from the mistake. This
    // reads every generated file, not just the ones a build would touch.
    const dir = scaffold({ name: "Placeholders", ios: true });
    const unfilled = files(dir).filter((rel) =>
      /\{\{[A-Z_]+\}\}/.test(readFileSync(join(dir, rel), "utf8")),
    );
    expect(unfilled).toEqual([]);
  });

  test("--desktop writes neither mobile shell", () => {
    const dir = scaffold({ name: "Desk Only", desktop: true });
    expect(existsSync(join(dir, "native", "src", "main.rs"))).toBe(true);
    expect(existsSync(join(dir, "android"))).toBe(false);
    expect(existsSync(join(dir, "ios"))).toBe(false);
  });

  test("--ios adds the iOS shell, which is otherwise opt-in", () => {
    expect(existsSync(join(scaffold({ name: "No Ios" }), "ios"))).toBe(false);
    expect(existsSync(join(scaffold({ name: "With Ios", ios: true }), "ios", "project.yml"))).toBe(true);
  });

  test("--mobile and --desktop ask for opposite things and are refused", () => {
    const root = mkdtempSync(join(tmpdir(), "voussoir-init-"));
    roots.push(root);
    const dir = join(root, "app");
    const error = spyOn(console, "error").mockImplementation(() => {});
    try {
      const args: Args = {
        command: "init",
        positionals: [dir],
        flags: { mobile: true, desktop: true },
        help: false,
      };
      expect(init(args)).toBe(1);
    } finally {
      error.mockRestore();
    }
    expect(existsSync(dir)).toBe(false);
  });

  test("the next steps mention only the shells it wrote", () => {
    // The hints used to list `android build` and `ios build` unconditionally,
    // so a desktop-only scaffold told the user to run a command for a tree that
    // was not there.
    const desktop = scaffoldOutput({ name: "Desk Hints", desktop: true });
    expect(desktop).not.toContain("android build");
    expect(desktop).not.toContain("ios build");

    const everything = scaffoldOutput({ name: "All Hints", ios: true });
    expect(everything).toContain("android build");
    expect(everything).toContain("ios build");
  });
});
