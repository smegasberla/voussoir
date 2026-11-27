/** `voussoir init [dir]`: write a project.
 *
 *  Templates are inlined into the CLI binary at compile time, so an installed
 *  `voussoir` has no asset directory to lose, and a scaffolded project contains no
 *  generated code: everything under `android/` is either configuration you may
 *  edit or source you own.
 */

import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { voussoirAndroidDependency, voussoirDependency, voussoirIosDependency } from "../framework.ts";
import { boolFlag, stringFlag, type Args } from "../args.ts";
import { humanize, identifiers } from "../naming.ts";

import voussoirJson from "../../template/voussoir.json.tmpl" with { type: "text" };
import cargoToml from "../../template/Cargo.toml.tmpl" with { type: "text" };
import mainRs from "../../template/main.rs.tmpl" with { type: "text" };
import assetsRs from "../../template/assets.rs.tmpl" with { type: "text" };
import indexHtml from "../../template/index.html.tmpl" with { type: "text" };
import mainTs from "../../template/main.ts.tmpl" with { type: "text" };
import styleCss from "../../template/style.css.tmpl" with { type: "text" };
import tsconfig from "../../template/tsconfig.json.tmpl" with { type: "text" };
import voussoirDts from "../../template/voussoir.d.ts.tmpl" with { type: "text" };
import gitignore from "../../template/gitignore.tmpl" with { type: "text" };

import androidBuildGradle from "../../template/android/build.gradle.tmpl" with { type: "text" };
import androidSettings from "../../template/android/settings.gradle.tmpl" with { type: "text" };
import androidProps from "../../template/android/gradle.properties.tmpl" with { type: "text" };
import androidManifest from "../../template/android/AndroidManifest.xml.tmpl" with { type: "text" };
import androidThemes from "../../template/android/themes.xml.tmpl" with { type: "text" };
import androidActivity from "../../template/android/MainActivity.java.tmpl" with { type: "text" };
import androidDialogs from "../../template/android/VoussoirDialogs.java.tmpl" with { type: "text" };
import androidMenu from "../../template/android/VoussoirMenu.java.tmpl" with { type: "text" };
import androidTray from "../../template/android/VoussoirTray.java.tmpl" with { type: "text" };
import androidHttp from "../../template/android/VoussoirHttp.java.tmpl" with { type: "text" };
import androidMenuTest from "../../template/android/androidTest/VoussoirMenuTest.java.tmpl" with { type: "text" };
import androidDialogsTest from "../../template/android/androidTest/VoussoirDialogsTest.java.tmpl" with { type: "text" };
import androidLifecycleTest from "../../template/android/androidTest/VoussoirLifecycleTest.java.tmpl" with { type: "text" };
import androidDeviceSupport from "../../template/android/androidTest/VoussoirDeviceSupport.java.tmpl" with { type: "text" };
import androidDeviceTest from "../../template/android/androidTest/VoussoirDeviceTest.java.tmpl" with { type: "text" };
import androidCargo from "../../template/android/Cargo.toml.tmpl" with { type: "text" };
import androidLib from "../../template/android/lib.rs.tmpl" with { type: "text" };
import androidAssets from "../../template/android/assets.rs.tmpl" with { type: "text" };

import iosCargo from "../../template/ios/Cargo.toml.tmpl" with { type: "text" };
import iosLib from "../../template/ios/lib.rs.tmpl" with { type: "text" };
import iosAssets from "../../template/ios/assets.rs.tmpl" with { type: "text" };
import iosProjectYml from "../../template/ios/project.yml.tmpl" with { type: "text" };
import iosPlist from "../../template/ios/Info.plist.tmpl" with { type: "text" };
import iosMainSwift from "../../template/ios/main.swift.tmpl" with { type: "text" };
import iosBridge from "../../template/ios/VoussoirBridge.h.tmpl" with { type: "text" };

import commandsCargo from "../../template/commands/Cargo.toml.tmpl" with { type: "text" };
import commandsLib from "../../template/commands/src/lib.rs.tmpl" with { type: "text" };

export function init(args: Args): number {
  const dir = resolve(args.positionals[0] ?? ".");
  const force = boolFlag(args, "force");
  // A mobile-only project keeps the frontend and the Android shell and skips the
  // desktop crate entirely: an app that only ever ships to Play has no use for
  // a `native/` tree it will never build, and carrying one invites the two
  // command lists to drift apart.
  const mobile = boolFlag(args, "mobile");
  const desktopOnly = boolFlag(args, "desktop");
  // iOS needs Xcode and only runs on macOS, so it is opt-in. Shipping an `ios/`
  // tree nobody will ever build is the bloat this project exists to argue
  // against, and it is the largest share of a scaffold's files.
  const withIos = boolFlag(args, "ios");

  if (mobile && desktopOnly) {
    console.error("voussoir: `--mobile` and `--desktop` ask for opposite things");
    return 1;
  }
  const name = stringFlag(args, "name") ?? humanize(basename(dir));
  const ids = identifiers(name, stringFlag(args, "package"));

  if (existsSync(dir) && readdirSync(dir).length > 0 && !force) {
    console.error(`voussoir: ${dir} is not empty (pass --force to write anyway)`);
    return 1;
  }

  const values: Record<string, string> = {
    NAME: ids.display,
    CRATE: ids.crate,
    LIB: ids.lib,
    PACKAGE: ids.package,
    JNI_PACKAGE: ids.jniPackage,
    DOMAIN: ids.domain,
    LEAF: ids.leaf,
    VOUSSOIR_PATH: voussoirDependency(),
    VOUSSOIR_PATH_INNER: voussoirDependency(),
    VOUSSOIR_ANDROID_PATH_INNER: voussoirAndroidDependency(),
    VOUSSOIR_IOS_PATH_INNER: voussoirIosDependency(),
  };

  // Every placeholder in one pass, so a template can reference any of them and a
  // missing value shows up as a literal {{X}} in the output rather than silently.
  const fill = (template: string) =>
    template.replace(/\{\{([A-Z_]+)\}\}/g, (whole, key: string) => values[key] ?? whole);

  const write = (path: string, contents: string) => {
    const full = join(dir, path);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, contents);
    console.log(`  created ${path}`);
  };

  console.log(`\nScaffolding ${ids.display} in ${dir}\n`);

  // Shared by desktop and Android: one frontend, one config, one set of commands.
  write("voussoir.json", fill(voussoirJson));
  write("index.html", fill(indexHtml));
  write("src/main.ts", fill(mainTs));
  write("src/style.css", fill(styleCss));
  write("src/voussoir.d.ts", voussoirDts);
  write("tsconfig.json", fill(tsconfig));
  write(".gitignore", gitignore);

  // Desktop shell. A `--mobile` project stops at the Android shell below.
  if (!mobile) {
    write("native/Cargo.toml", fill(cargoToml));
    write("native/src/main.rs", fill(mainRs));
    write("native/src/assets.rs", fill(assetsRs));
    write("native/commands/Cargo.toml", fill(commandsCargo));
    write("native/commands/src/lib.rs", fill(commandsLib));
  }

  // Android shell. Same app, different entry point and packaging. `--desktop`
  // skips it: a desktop-only app has no use for a Gradle project it will never
  // build, and this tree is most of the files in a scaffold.
  if (!desktopOnly) {
    write("android/Cargo.toml", fill(androidCargo));
    write("android/src/lib.rs", fill(androidLib));
    write("android/src/assets.rs", fill(androidAssets));
    write("android/app/build.gradle", fill(androidBuildGradle));
    write("android/app/settings.gradle", fill(androidSettings));
    write("android/app/gradle.properties", fill(androidGradleProps(androidProps)));
    write("android/app/src/main/AndroidManifest.xml", fill(androidManifest));
    write("android/app/src/main/res/values/themes.xml", fill(androidThemes));
    const java = `android/app/src/main/java/${ids.package.replace(/\./g, "/")}`;
    write(`${java}/MainActivity.java`, fill(androidActivity));
    write(`${java}/VoussoirDialogs.java`, fill(androidDialogs));
    write(`${java}/VoussoirMenu.java`, fill(androidMenu));
    write(`${java}/VoussoirTray.java`, fill(androidTray));
    write(`${java}/VoussoirHttp.java`, fill(androidHttp));

    // Instrumentation tests, in two layers. The first asserts pure data
    // translation, which needs no window: menu ids, intent construction, MIME
    // mapping. The second drives the real round trips on a device, which is the
    // only place a `content://` URI becoming a path, a notification being posted, or
    // a TLS request through the system trust store can be observed at all.
    // `VoussoirDeviceSupport` is shared plumbing for both, not a test.
    const androidTest = `android/app/src/androidTest/java/${ids.package.replace(/\./g, "/")}`;
    write(`${androidTest}/VoussoirMenuTest.java`, fill(androidMenuTest));
    write(`${androidTest}/VoussoirDialogsTest.java`, fill(androidDialogsTest));
    write(`${androidTest}/VoussoirLifecycleTest.java`, fill(androidLifecycleTest));
    write(`${androidTest}/VoussoirDeviceSupport.java`, fill(androidDeviceSupport));
    write(`${androidTest}/VoussoirDeviceTest.java`, fill(androidDeviceTest));
  }

  // iOS shell. Same app a third time: Swift's `main.swift` calls the staticlib's
  // entry as the first thing the process does, and XcodeGen generates the
  // .xcodeproj from the committed project.yml, which is the file you own.
  // Opt-in (`--ios`), because it needs Xcode and only builds on macOS.
  if (!desktopOnly && withIos) {
    write("ios/Cargo.toml", fill(iosCargo));
    write("ios/src/lib.rs", fill(iosLib));
    write("ios/src/assets.rs", fill(iosAssets));
    write("ios/project.yml", fill(iosProjectYml));
    write("ios/app/Info.plist", fill(iosPlist));
    write("ios/app/main.swift", fill(iosMainSwift));
    write("ios/app/VoussoirBridge.h", fill(iosBridge));
  }

  // `cd .` is what an in-place scaffold would otherwise print, which is not an
  // instruction. Left out when the project was written into the current directory.
  const target = args.positionals[0] ?? ".";
  const next = ["", "Next:"];
  if (target !== ".") {
    next.push(`  cd ${target}`);
  }
  if (!mobile) {
    next.push("  voussoir dev              # desktop, with hot reload");
    next.push("  voussoir build --size     # desktop release binary");
  }
  if (!desktopOnly) {
    next.push("  voussoir android build    # APK");
    next.push("  voussoir android build --aab  # Play-ready bundle");
    next.push("  voussoir android run      # install and launch on a device or emulator");
  }
  if (!desktopOnly && withIos) {
    next.push("  voussoir ios build        # simulator app (macOS with Xcode)");
    next.push("  voussoir ios run          # install and launch on a booted simulator");
  }
  next.push("");
  console.log(next.join("\n"));
  return 0;
}

/** gradle.properties has no placeholders of its own; kept as a function so the
 *  import is not flagged unused when the file is written verbatim. */
function androidGradleProps(template: string): string {
  return template;
}
