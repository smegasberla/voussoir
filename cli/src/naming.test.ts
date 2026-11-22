import { describe, expect, test } from "bun:test";
import { crateName, identifiers, jniPackage, libName } from "./naming.ts";

describe("crateName", () => {
  test("slugifies human names", () => {
    expect(crateName("My Notes")).toBe("my-notes");
    expect(crateName("  Spaces  ")).toBe("spaces");
    expect(crateName("a.b/c")).toBe("a-b-c");
  });

  test("never produces an empty or digit-leading crate", () => {
    expect(crateName("")).toBe("voussoir-app");
    expect(crateName("!!!")).toBe("voussoir-app");
    expect(crateName("3d viewer")).toBe("app-3d-viewer");
  });
});

describe("libName", () => {
  test("uses underscores", () => {
    expect(libName("my-notes")).toBe("my_notes");
  });

  test("avoids Rust keywords", () => {
    // `mod` is a keyword; a lib named `mod` would not compile.
    expect(libName("mod")).toBe("mod_app");
    expect(libName("type")).toBe("type_app");
  });
});

describe("jniPackage", () => {
  test("mangles dots and underscores the way JNI does", () => {
    expect(jniPackage("com.voussoir.app")).toBe("com_voussoir_app");
    // Underscores become _1, otherwise the JNI decoder reads them as dots.
    expect(jniPackage("com.voussoir.my_notes")).toBe("com_voussoir_my_1notes");
  });
});

describe("identifiers", () => {
  test("splits the package for tao and wry", () => {
    const ids = identifiers("My Notes");
    expect(ids.package).toBe("com.voussoir.my_notes");
    expect(ids.domain).toBe("com_voussoir");
    expect(ids.leaf).toBe("my_notes");
    expect(ids.jniPackage).toBe("com_voussoir_my_1notes");
    expect(ids.lib).toBe("my_notes");
  });

  test("honours a package override", () => {
    const ids = identifiers("Notes", "dev.example.notes");
    expect(ids.package).toBe("dev.example.notes");
    expect(ids.domain).toBe("dev_example");
    expect(ids.leaf).toBe("notes");
  });

  test("refuses a package the Android toolchain cannot accept", () => {
    // Each of these used to reach the Gradle namespace and the JNI symbols and
    // then fail at launch, where the cause is a mile from the name.
    expect(() => identifiers("Notes", "notes")).toThrow(/package name/);
    expect(() => identifiers("Notes", "Notes.App")).toThrow(/lowercase/);
    expect(() => identifiers("Notes", "com.exa mple")).toThrow(/lowercase/);
    expect(() => identifiers("Notes", "com.example.")).toThrow(/lowercase/);
    expect(() => identifiers("Notes", "com.example.class")).toThrow(/keyword/);
  });

  test("keeps a reviewed lowercase package untouched", () => {
    const ids = identifiers("Notes", "dev.example.notes_2");
    expect(ids.package).toBe("dev.example.notes_2");
  });
});
