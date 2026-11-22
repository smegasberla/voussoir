/** `voussoir android <build|run|verify>`: the Android half of a Voussoir project.
 *
 *  Kept separate from `voussoir build` because the toolchain is entirely different:
 *  cross-compile with the NDK, stage the `.so`, then let Gradle package it. The
 *  project already contains its Gradle files, so nothing here generates
 *  configuration at build time.
 */

import { existsSync, mkdirSync, copyFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";
import { bundle, writeAssetTable } from "../pipeline.ts";
import { requireProject, type Project } from "../project.ts";
import { checkParity } from "../parity.ts";
import { run } from "../proc.ts";

/** ABIs Voussoir builds, and the Rust target + NDK compiler prefix for each. */
const ABIS: Record<string, { target: string; clang: string }> = {
  "arm64-v8a": { target: "aarch64-linux-android", clang: "aarch64-linux-android" },
  "x86_64": { target: "x86_64-linux-android", clang: "x86_64-linux-android" },
  "armeabi-v7a": { target: "armv7-linux-androideabi", clang: "armv7a-linux-androideabi" },
};

export const DEFAULT_API = 26;

export function androidDir(project: Project): string {
  return join(project.root, "android");
}

/** NDK host prebuilt directories, one per desktop OS. */
export const NDK_HOST_PREBUILT: Record<string, string> = {
  linux: "linux-x86_64",
  darwin: "darwin-x86_64",
  win32: "windows-x86_64",
};

/** The NDK prebuilt directory for a host, from what the NDK actually contains.
 *
 *  Derived rather than hardcoded: the NDK names its toolchains by host OS
 *  (`linux-x86_64`, `darwin-x86_64`, `windows-x86_64`, and `darwin-arm64` on
 *  newer releases), so a fixed path works on one machine and breaks the
 *  Android build on the other two. `entries` is the listing of the NDK's
 *  `prebuilt` directory; when it names a directory for this host, that wins.
 */
export function ndkHostDir(platform: string = process.platform, entries: string[] = []): string {
  const prefix =
    platform === "win32" ? "windows-" : platform === "darwin" ? "darwin-" : "linux-";
  const match = entries.find((entry) => entry.startsWith(prefix));
  return match ?? NDK_HOST_PREBUILT[platform] ?? NDK_HOST_PREBUILT.linux!;
}

/** The NDK toolchain, or a message saying what to install. */
export function ndkToolchain(ndk: string | undefined): string {
  if (!ndk) {
    throw new Error(
      "no Android NDK found. Set ANDROID_NDK_HOME, or install one with\n" +
        "  sdkmanager 'ndk;27.0.12077973'",
    );
  }
  const prebuilt = join(ndk, "toolchains", "llvm", "prebuilt");
  const entries = existsSync(prebuilt) ? readdirSync(prebuilt) : [];
  const bin = join(prebuilt, ndkHostDir(process.platform, entries), "bin");
  if (!existsSync(bin)) {
    throw new Error(`NDK at ${ndk} has no llvm toolchain for this host (looked in ${bin})`);
  }
  return bin;
}

/** Read `[lib] name` from the Android crate so the `.so` name is never guessed. */
export function androidLibName(project: Project): string {
  const manifest = join(androidDir(project), "Cargo.toml");
  if (!existsSync(manifest)) {
    throw new Error(`no Android crate at ${manifest}; run voussoir init in this project`);
  }
  const text = require("node:fs").readFileSync(manifest, "utf8") as string;
  const match = /\[lib\][\s\S]*?name\s*=\s*"([^"]+)"/.exec(text);
  if (!match) {
    throw new Error(`${manifest} has no [lib] name; the APK has nothing to load`);
  }
  return match[1]!;
}

export async function build(args: Args): Promise<number> {
  const project = requireProject();
  const abi = stringFlag(args, "abi") ?? "arm64-v8a";
  const abiSpec = ABIS[abi];
  if (!abiSpec) {
    throw new Error(`unknown --abi ${abi} (choose ${Object.keys(ABIS).join(", ")})`);
  }

  const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
  const ndk = process.env.ANDROID_NDK_HOME ?? process.env.ANDROID_NDK_ROOT;
  const toolchain = ndkToolchain(ndk);
  const lib = androidLibName(project);
  const android = androidDir(project);
  const api = Number(stringFlag(args, "api") ?? DEFAULT_API);

  const clang = join(toolchain, `${abiSpec.clang}${api}-clang`);
  if (!existsSync(clang)) throw new Error(`no compiler at ${clang}`);

  // A command registered on only one shell fails at runtime on the other, with
  // "unknown command". Catch it at build time instead.
  const parity = checkParity(project.root);
  if (!parity.ok) {
    console.error(`voussoir: command parity problem between desktop and android\n  ${parity.message}`);
    return 1;
  }
  console.log(`  ${parity.message}`);

  // The frontend must be bundled before it can be embedded.
  console.log(`voussoir android build: ${project.config.name} (${abi})`);
  await bundle(project, true);
  writeAssetTable(project, android);

  const env = {
    ...process.env,
    ANDROID_HOME: sdk,
    ANDROID_NDK_HOME: ndk,
    // wry's build script generates the Kotlin classes the app compiles against.
    WRY_ANDROID_PACKAGE: readPackage(project),
    WRY_ANDROID_LIBRARY: lib,
    WRY_ANDROID_KOTLIN_FILES_OUT_DIR: kotlinOutDir(project),
    [`CC_${abiSpec.target.replace(/-/g, "_")}`]: clang,
    [`AR_${abiSpec.target.replace(/-/g, "_")}`]: join(toolchain, "llvm-ar"),
    [`CARGO_TARGET_${abiSpec.target.replace(/-/g, "_").toUpperCase()}_LINKER`]: clang,
  };

  await run(["cargo", "build", "--release", "--target", abiSpec.target], {
    cwd: android,
    env,
  });

  const so = join(
    android,
    "target",
    abiSpec.target,
    "release",
    `lib${lib}.so`,
  );
  if (!statSync(so, { throwIfNoEntry: false })) {
    throw new Error(`cargo finished but ${so} is missing`);
  }

  const jni = join(android, "app", "src", "main", "jniLibs", abi);
  mkdirSync(jni, { recursive: true });
  copyFileSync(so, join(jni, `lib${lib}.so`));
  console.log(`  staged ${abi}/lib${lib}.so`);

  // `--release` produces a release APK, `--aab` an Android App Bundle for Play.
  // Both go through the release variant, which the template signs with a real
  // keystore when one is configured and with the debug key otherwise.
  const wantAab = boolFlag(args, "aab");
  const wantRelease = boolFlag(args, "release") || wantAab;
  const task = wantAab ? "bundleRelease" : wantRelease ? "assembleRelease" : "assembleDebug";

  await run(["gradle", task, "--no-daemon", "-q"], { cwd: join(android, "app") });

  const produced: string[] = [];
  const apk = wantAab ? null : findApk(project, wantRelease);
  if (apk) produced.push(apk);
  if (wantAab) {
    const aab = findBundle(project);
    if (aab) produced.push(aab);
  }
  if (produced.length === 0) {
    throw new Error(`Gradle finished but ${task} produced no artifact`);
  }
  console.log("");
  for (const artifact of produced) {
    const mb = (statSync(artifact).size / 1048576).toFixed(1);
    console.log(`  ${artifact} (${mb} MB)`);
  }
  return 0;
}

export async function runOnDevice(args: Args): Promise<number> {
  const project = requireProject();
  const adb = adbPath();

  // Prefer the debug build, but launch a release APK too when that is what was
  // built last: `voussoir android build --release` followed by `run` is the obvious
  // pairing, and refusing it would be an artificial restriction.
  const apk = findApk(project) ?? findApk(project, true);
  if (!apk) {
    console.error("voussoir: no APK yet; run `voussoir android build` first");
    return 1;
  }

  const pkg = readPackage(project);
  console.log(`voussoir android run: ${pkg}`);
  await run([adb, "install", "-r", apk], { silent: true });
  await run([adb, "shell", "am", "force-stop", pkg], { silent: true });
  await run([adb, "logcat", "-c"], { silent: true });
  await run([
    adb,
    "shell",
    "am",
    "start",
    "-n",
    `${pkg}/.MainActivity`,
  ], { silent: true });

  console.log("  launched. logs:\n");
  const logs = Bun.spawn([adb, "logcat", "-s", "Voussoir:*", "RustStdoutStderr:*"], {
    stdout: "inherit",
    stderr: "inherit",
  });
  process.on("SIGINT", () => {
    logs.kill();
    process.exit(0);
  });
  await logs.exited;
  return boolFlag(args, "wait") ? 0 : 0;
}

/** Locate the built APK without hardcoding the Gradle output layout beyond what
 *  is documented. `release` picks the release variant's directory so a stale
 *  debug APK from an earlier run cannot be reported as the new artifact. */
export function findApk(project: Project, release = false): string | null {
  return newest(
    join(androidDir(project), "app", "build", "outputs", "apk", release ? "release" : "debug"),
    ".apk",
  );
}

/** The release AAB, the artifact Play Store uploads consume. */
export function findBundle(project: Project): string | null {
  return newest(join(androidDir(project), "app", "build", "outputs", "bundle", "release"), ".aab");
}

/** Newest file with `suffix` under `base`, or null when the directory is absent. */
function newest(base: string, suffix: string): string | null {
  if (!existsSync(base)) return null;
  let best: { path: string; mtime: number } | null = null;
  const stack = [base];
  while (stack.length > 0) {
    const dir = stack.pop()!;
    for (const entry of require("node:fs").readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.name.endsWith(suffix)) {
        const mtime = statSync(full).mtimeMs;
        if (!best || mtime > best.mtime) best = { path: full, mtime };
      }
    }
  }
  return best ? best.path : null;
}

/** The Android package, read from the generated Gradle file. */
export function readPackage(project: Project): string {
  const gradle = join(androidDir(project), "app", "build.gradle");
  const text = existsSync(gradle) ? require("node:fs").readFileSync(gradle, "utf8") : "";
  const match = /namespace\s+['"]([^'"]+)['"]/.exec(text as string);
  if (!match) throw new Error(`${gradle} has no namespace; cannot derive the package`);
  return match[1]!;
}

/** Where wry should write its generated Kotlin: beside the package's sources. */
function kotlinOutDir(project: Project): string {
  const pkg = readPackage(project);
  return join(
    androidDir(project),
    "app",
    "src",
    "main",
    "java",
    ...pkg.split("."),
  );
}

function adbPath(): string {
  const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT ?? "";
  const candidate = join(sdk, "platform-tools", "adb");
  if (existsSync(candidate)) return candidate;
  const found = Bun.which("adb");
  if (found) return found;
  throw new Error("no adb found. Install platform-tools or set ANDROID_HOME");
}

/** Write a `.gitignore` entry list for the Android tree, so a scaffolded project
 *  never commits build output or generated Kotlin. */
export function androidGitignore(): string {
  return [
    "build/",
    ".gradle/",
    "target/",
    "app/src/main/jniLibs/",
    "app/src/main/java/**/*.kt",
    "app/src/main/java/**/proguard-wry.pro",
    "local.properties",
    "",
  ].join("\n");
}

/** Used by `voussoir init`; exported so a test can assert the shape. */
export function writeAndroidGitignore(project: Project): void {
  writeFileSync(join(androidDir(project), ".gitignore"), androidGitignore());
}
