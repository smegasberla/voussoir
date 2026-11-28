/** `voussoir doctor`: environment and project checks, without spawning a build.
 *
 *  Every check reads something real (a binary on PATH, a file, a manifest) so a
 *  green report means the next command will actually work.
 */

import { existsSync } from "node:fs";
import { join } from "node:path";
import { boolFlag, type Args } from "../args.ts";
import { checkFile } from "../a11y.ts";
import { findProject } from "../project.ts";

/** `note` is neither a pass nor a failure: the project check has no answer
 *  before `init`, and failing there tells a first-time user their toolchain is
 *  broken when the only thing missing is a directory of their own. */
type Check = { name: string; ok: boolean; detail: string; note?: boolean };

const BUN_VERSION = /^(\d+)\.(\d+)\./;

export async function doctor(args: Args): Promise<number> {
  const checks: Check[] = [];

  const bun = Bun.which("bun");
  checks.push({
    name: "bun",
    ok: Boolean(bun) && (Bun.version.match(BUN_VERSION)?.[1] ?? "0") >= "1",
    detail: bun ? `${bun} (${Bun.version})` : "not found on PATH",
  });

  const cargo = Bun.which("cargo");
  checks.push({
    name: "cargo",
    ok: Boolean(cargo),
    detail: cargo ?? "not found on PATH; install Rust",
  });

  // Linux and the BSDs both use GTK + WebKit2GTK; the package names differ, so the
  // hint says which one to install rather than a generic message.
  const gtkPlatform = ["linux", "freebsd", "netbsd", "openbsd", "dragonfly"].includes(
    process.platform,
  );
  if (gtkPlatform) {
    const webkit = hasWebkit();
    checks.push({
      name: "system webview",
      ok: webkit,
      detail: webkit
        ? "webkit2gtk found"
        : `webkit2gtk not found (${webkitPackageHint(process.platform)})`,
    });
    checks.push({
      name: "pkg-config",
      ok: Boolean(Bun.which("pkg-config")),
      detail: Bun.which("pkg-config") ?? "missing; needed to link GTK and WebKit",
    });
  } else {
    checks.push({
      name: "system webview",
      ok: true,
      detail: process.platform === "android" ? "provided by the system WebView" : "built in",
    });
  }

  // Android is a separate toolchain, so report it only when the project has one.
  const project = findProject();
  if (project && existsSync(join(project.root, "android", "Cargo.toml"))) {
    const ndk = process.env.ANDROID_NDK_HOME ?? process.env.ANDROID_NDK_ROOT;
    checks.push({
      name: "android NDK",
      ok: Boolean(ndk) && existsSync(ndk!),
      detail: ndk ?? "ANDROID_NDK_HOME not set (only needed for voussoir android)",
    });
    const adb = Bun.which("adb");
    checks.push({
      name: "adb",
      ok: Boolean(adb),
      detail: adb ?? "not on PATH (only needed for voussoir android run)",
    });
  }

  checks.push({
    name: "project",
    ok: Boolean(project),
    note: !project,
    detail: project
      ? project.root
      : "no voussoir.json here yet; everything above is what an app needs",
  });

  if (project) {
    const entry = join(project.root, project.config.entry);
    checks.push({
      name: "entry html",
      ok: existsSync(entry),
      detail: existsSync(entry) ? project.config.entry : `missing: ${entry}`,
    });
    const manifest = join(project.root, "native", "Cargo.toml");
    checks.push({
      name: "native crate",
      ok: existsSync(manifest),
      detail: existsSync(manifest) ? "native/Cargo.toml" : "missing: native/Cargo.toml",
    });

    // Accessibility of the markup, because the webview's tree is built from it and
    // a missing label is invisible until a reader hits it. A finding is a note
    // rather than a failure: an app can be shipping and still have one, and
    // `doctor` reports rather than blocks. It is not silent, either.
    const findings = checkFile(entry);
    checks.push({
      name: "a11y markup",
      ok: findings.length === 0,
      note: findings.length > 0,
      detail:
        findings.length === 0
          ? "labelled controls, ordered headings, a landmark"
          : `${findings.length} finding(s): ${findings.map((f) => f.rule).join(", ")}`,
    });
  }

  if (boolFlag(args, "json")) {
    console.log(JSON.stringify({ ok: passed(checks), checks }, null, 2));
  } else {
    for (const check of checks) {
      const label = check.ok ? "ok  " : check.note ? "note" : "FAIL";
      console.log(`${label} ${check.name.padEnd(14)} ${check.detail}`);
    }
    if (!project) {
      console.log("\nNext: voussoir init         # scaffold an app in this directory");
    }
    // The findings themselves, with a line number, only when there are some: a
    // green run should stay one line long.
    const a11y = checks.find((check) => check.name === "a11y markup");
    if (project && a11y && !a11y.ok) {
      console.log("");
      for (const finding of checkFile(join(project.root, project.config.entry))) {
        console.log(`  ${project.config.entry}:${finding.line}  ${finding.message}`);
      }
    }
  }

  return passed(checks) ? 0 : 1;
}

/** A note is not a failure, so it must not decide the exit code. */
function passed(checks: Check[]): boolean {
  return checks.every((check) => check.ok || check.note);
}

/** The package name for WebKit2GTK on this platform. */
export function webkitPackageHint(platform: string): string {
  switch (platform) {
    case "freebsd":
      return "pkg install webkit2-gtk_41";
    case "netbsd":
    case "openbsd":
    case "dragonfly":
      return "install webkit2gtk from pkgsrc";
    default:
      return "install webkit2gtk-4.1";
  }
}

function hasWebkit(): boolean {
  const pkgconfig = Bun.which("pkg-config");
  if (!pkgconfig) return false;
  const proc = Bun.spawnSync(["pkg-config", "--exists", "webkit2gtk-4.1"]);
  if (proc.exitCode === 0) return true;
  const legacy = Bun.spawnSync(["pkg-config", "--exists", "webkit2gtk-4.0"]);
  return legacy.exitCode === 0;
}
