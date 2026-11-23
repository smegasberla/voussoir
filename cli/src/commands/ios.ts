/** `voussoir ios <build|run|verify>`: the iOS half of a Voussoir project.
 *
 *  The toolchain is entirely different from Android's: a Rust staticlib that
 *  Xcode links, instead of a `.so` an activity loads. Everything else is shared
 *  with the other shells, so the work here is bundling, cross-compiling,
 *  staging the library, generating the Xcode project from its committed
 *  `project.yml`, and driving `xcodebuild` and `simctl`.
 *
 *  Device builds and App Store signing are out of scope on purpose: a device
 *  build compiles, but installing it needs a signed provisioning profile, and
 *  that belongs to whoever owns the Apple account, not to a scaffold.
 */

import { existsSync, mkdirSync, copyFileSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";
import { bundle, writeAssetTable, withReleaseSize } from "../pipeline.ts";
import { requireProject, type Project } from "../project.ts";
import { checkParity } from "../parity.ts";
import { run } from "../proc.ts";

/** Targets Voussoir builds, and the SDK `xcodebuild` uses for each. */
const TARGETS: Record<string, { target: string; sdk: string; products: string }> = {
  sim: { target: "aarch64-apple-ios-sim", sdk: "iphonesimulator", products: "Debug-iphonesimulator" },
  device: { target: "aarch64-apple-ios", sdk: "iphoneos", products: "Debug-iphoneos" },
};

export function iosDir(project: Project): string {
  return join(project.root, "ios");
}

/** Read `[lib] name` from the iOS crate so the `.a` name is never guessed. */
export function iosLibName(project: Project): string {
  const manifest = join(iosDir(project), "Cargo.toml");
  if (!existsSync(manifest)) {
    throw new Error(`no iOS crate at ${manifest}; run voussoir init in this project`);
  }
  const text = readFileSync(manifest, "utf8");
  const match = /\[lib\][\s\S]*?name\s*=\s*"([^"]+)"/.exec(text);
  if (!match) {
    throw new Error(`${manifest} has no [lib] name; the app has nothing to link`);
  }
  return match[1]!;
}

/** The XcodeGen target/scheme name, read from the committed project.yml.
 *
 *  `voussoir init` writes `name: <crate>-iOS` once at the top and uses it for both
 *  the target and the scheme, so the first `name:` line is the single source. */
export function readScheme(project: Project): string {
  const file = join(iosDir(project), "project.yml");
  const text = existsSync(file) ? readFileSync(file, "utf8") : "";
  const match = /^name:\s*(\S+)/m.exec(text);
  if (!match) throw new Error(`${file} has no name; cannot derive the scheme`);
  return match[1]!;
}

/** The bundle identifier, read from the generated project.yml. */
function readBundle(project: Project): string {
  const file = join(iosDir(project), "project.yml");
  const text = existsSync(file) ? readFileSync(file, "utf8") : "";
  const match = /PRODUCT_BUNDLE_IDENTIFIER:\s*(\S+)/.exec(text);
  if (!match) throw new Error(`${file} has no PRODUCT_BUNDLE_IDENTIFIER`);
  return match[1]!;
}

export async function build(args: Args): Promise<number> {
  const project = requireProject();
  const variant = stringFlag(args, "target") === "device" ? "device" : "sim";
  const spec = TARGETS[variant]!;
  const lib = iosLibName(project);
  const ios = iosDir(project);

  // A command registered on only one shell fails at runtime on the other, with
  // "unknown command". Catch it at build time instead.
  const parity = checkParity(project.root);
  if (!parity.ok) {
    console.error(`voussoir: command parity problem between the shells\n  ${parity.message}`);
    return 1;
  }
  console.log(`  ${parity.message}`);

  // The frontend must be bundled before it can be embedded.
  console.log(`voussoir ios build: ${project.config.name} (${variant})`);
  await bundle(project, true);
  writeAssetTable(project, ios);

  const env = { ...process.env };

  await run(withReleaseSize(["cargo", "build", "--release", "--target", spec.target]), {
    cwd: ios,
    env,
  });

  const archive = join(ios, "target", spec.target, "release", `lib${lib}.a`);
  if (!statSync(archive, { throwIfNoEntry: false })) {
    throw new Error(`cargo finished but ${archive} is missing`);
  }
  const libDir = join(ios, "lib");
  mkdirSync(libDir, { recursive: true });
  copyFileSync(archive, join(libDir, `lib${lib}.a`));
  console.log(`  staged lib/lib${lib}.a`);

  // XcodeGen turns the committed project.yml into the .xcodeproj. The project
  // file itself is generated output (gitignored), so a stale one can never
  // disagree with the source of truth.
  if (Bun.which("xcodegen")) {
    await run(["xcodegen", "generate"], { cwd: ios, silent: true });
  } else if (!existsSync(join(ios, `${readScheme(project)}.xcodeproj`))) {
    throw new Error(
      "no xcodegen found and no generated .xcodeproj either. Install one with\n" +
        "  brew install xcodegen",
    );
  }

  // Two things a bare `-sdk iphonesimulator` gets wrong on a runner: with no
  // destination xcodebuild picks one by itself and reports "multiple matching
  // destinations", and it happily targets x86_64 while the staged Rust lib
  // carries only the architecture cargo built for this host. Pin both, so the
  // link step sees the slice that actually exists.
  const simArch = process.arch === "arm64" ? "arm64" : "x86_64";
  await run(
    [
      "xcodebuild",
      "-project",
      join(ios, `${readScheme(project)}.xcodeproj`),
      "-scheme",
      readScheme(project),
      "-sdk",
      spec.sdk,
      "-destination",
      "generic/platform=iOS Simulator",
      "-configuration",
      "Debug",
      "-derivedDataPath",
      join(ios, "build"),
      `ARCHS=${simArch}`,
      "CODE_SIGNING_ALLOWED=NO",
      "CODE_SIGN_IDENTITY=",
      "build",
    ],
    { cwd: ios },
  );

  const app = findApp(project, variant);
  if (!app) {
    throw new Error("xcodebuild finished but no .app was produced");
  }
  console.log(`  ${app} (${(statSync(app).size / 1048576).toFixed(1)} MB)`);
  return 0;
}

/** Install and launch on a simulator, then tail the app's stdout.
 *
 *  `--console-pty` is the simulator's logcat: the Rust staticlib's stdout and
 *  the shell's own diagnostics arrive on the terminal it is attached to. */
export async function runOnDevice(args: Args): Promise<number> {
  const project = requireProject();
  const bundle = readBundle(project);
  const udid = await bootedSimulator();
  const app = findApp(project, "sim");
  if (!app) {
    console.error("voussoir: no simulator app yet; run `voussoir ios build` first");
    return 1;
  }

  console.log(`voussoir ios run: ${bundle}`);
  await run(["xcrun", "simctl", "install", udid, app], { silent: true });
  await run(["xcrun", "simctl", "terminate", udid, bundle], { silent: true }).catch(() => {});

  console.log("  launched. output:\n");
  const proc = Bun.spawn(
    ["xcrun", "simctl", "launch", "--console-pty", udid, bundle],
    { stdout: "inherit", stderr: "inherit" },
  );
  process.on("SIGINT", () => {
    proc.kill();
    process.exit(0);
  });
  await proc.exited;
  return 0;
}

/** Build (unless told not to), install, launch, and assert the round trip.
 *
 *  Two markers, both read from the app's own stdout through `--console-pty`,
 *  because on iOS there is no logcat: the console is the only observer that
 *  sees both sides. `VOUSSOIR_IOS_START` proves the staticlib loaded and Rust is
 *  running; `VOUSSOIR_IOS_ROUND_TRIP` is printed by the shell's `greet` command,
 *  so it only appears after the page has loaded, called a command, and had
 *  that command answered — the full transport, shim and registry loop. */
export async function verify(args: Args): Promise<number> {
  const project = requireProject();
  const bundle = readBundle(project);
  const udid = await bootedSimulator();

  if (!boolFlag(args, "no-build")) {
    const code = await build({ ...args, ["no-build" as never]: undefined } as Args);
    if (code !== 0) return code;
  }
  const app = findApp(project, "sim");
  if (!app) {
    console.error("voussoir: no simulator app; run `voussoir ios build` first");
    return 1;
  }

  await run(["xcrun", "simctl", "install", udid, app], { silent: true });
  await run(["xcrun", "simctl", "terminate", udid, bundle], { silent: true }).catch(() => {});

  await run(
    ["xcrun", "simctl", "spawn", udid, "launchctl", "setenv", "RUST_BACKTRACE", "1"],
    { silent: true },
  ).catch(() => {});
  const proc = Bun.spawn(["xcrun", "simctl", "launch", "--console-pty", udid, bundle], {
    stdout: "pipe",
  });
  // 90 s, not 45: the app is installed into a simulator that has just booted,
  // and the first launch pays for dyld and the webview's own start-up before the
  // page runs. A wedged launch still fails, just later.
  const timer = setTimeout(() => proc.kill(), 90000);
  let text = "";
  const reader = proc.stdout.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      text += new TextDecoder().decode(value);
      // Stop waiting as soon as the round trip lands; a healthy launch needs
      // seconds, and the timeout only exists for a wedged one.
      if (text.includes("VOUSSOIR_IOS_ROUND_TRIP")) {
        proc.kill();
        break;
      }
    }
  } finally {
    clearTimeout(timer);
  }

  const problems: string[] = [];
  if (!text.includes("VOUSSOIR_IOS_START")) {
    problems.push("the app never reached its Rust entry");
  }
  if (!text.includes("VOUSSOIR_IOS_ROUND_TRIP")) {
    problems.push("the page never completed a command round trip");
  }
  if (problems.length > 0) {
    console.error(`voussoir ios verify: ${problems.join("; ")}`);
    const tail = text.trimEnd().split("\n").slice(-20).join("\n");
    if (tail) console.error(tail);
    return 1;
  }
  console.log(`ok   ios: entry reached and page round trip completed (${bundle})`);
  return 0;
}

/** The UDID of a booted simulator, booting the first available one otherwise.
 *
 *  `simctl boot` returns before the OS is up, and an install into a still
 *  booting device fails; `bootstatus -b` is the documented way to wait. */
async function bootedSimulator(): Promise<string> {
  const listed = Bun.spawnSync(["xcrun", "simctl", "list", "devices"], {
    stdout: "pipe",
  });
  const lines = listed.stdout.toString().split("\n");
  for (const line of lines) {
    if (line.includes("(Booted)")) {
      const match = /\(([0-9A-Fa-f-]{36})\)/.exec(line);
      if (match) return match[1]!;
    }
  }
  for (const line of lines) {
    if (line.includes("unavailable")) continue;
    const match = /\(([0-9A-Fa-f-]{36})\)/.exec(line);
    if (match) {
      await run(["xcrun", "simctl", "boot", match[1]!], { silent: true });
      await run(["xcrun", "simctl", "bootstatus", match[1]!, "-b"], { silent: true });
      return match[1]!;
    }
  }
  throw new Error(
    "no iOS simulator available. Install Xcode and create one with\n" +
      "  xcrun simctl create 'iPhone 16' 'iPhone 16'",
  );
}

/** The built .app for a variant, or null when it has not been built. */
export function findApp(project: Project, variant: keyof typeof TARGETS): string | null {
  const spec = TARGETS[variant];
  if (!spec) return null;
  const base = join(iosDir(project), "build", "Build", "Products", spec.products);
  if (!existsSync(base)) return null;
  let best: string | null = null;
  let bestTime = 0;
  for (const entry of readdirSync(base, { withFileTypes: true })) {
    const full = join(base, entry.name);
    if (entry.isDirectory() && entry.name.endsWith(".app")) {
      const time = statSync(full).mtimeMs;
      if (!best || time > bestTime) {
        best = full;
        bestTime = time;
      }
    }
  }
  return best;
}
