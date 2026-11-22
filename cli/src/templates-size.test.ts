import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/** The scaffold is what every new app ships with; its release profile is the
 *  difference between a 2 MB download and a 4 MB one on every platform. */
describe("templates release profile", () => {
  test("desktop enables LTO, one codegen unit and stripping", () => {
    const manifest = readFileSync(join(import.meta.dir, "..", "template", "Cargo.toml.tmpl"), "utf8");
    expect(manifest).toContain('[profile.release]');
    for (const line of ['opt-level = "s"', "lto = true", "codegen-units = 1", "strip = true"]) {
      expect(manifest).toContain(line);
    }
  });

  test("devtools is not part of full", () => {
    const manifest = readFileSync(
      join(import.meta.dir, "..", "..", "voussoir", "Cargo.toml"),
      "utf8",
    );
    const full = /^full = \[(.*)\]$/m.exec(manifest);
    expect(full).not.toBeNull();
    expect(full![1]).not.toContain("devtools");
  });
});
