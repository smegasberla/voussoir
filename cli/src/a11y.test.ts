import { describe, expect, test } from "bun:test";
import { checkHtml } from "./a11y.ts";

const page = (body: string) => `<!doctype html><html><body>${body}</body></html>`;

describe("a11y: form controls", () => {
  test("an input with a for/id label passes", () => {
    expect(checkHtml(page(`<main><form><label for="n">Name</label><input id="n"></form></main>`))).toEqual(
      [],
    );
  });

  test("an input wrapped in its label passes", () => {
    expect(checkHtml(page(`<main><label>Name <input></label></main>`))).toEqual([]);
  });

  test("aria-label is a name", () => {
    expect(checkHtml(page(`<main><input aria-label="Search"></main>`))).toEqual([]);
  });

  test("aria-labelledby is a name", () => {
    expect(
      checkHtml(page(`<main><span id="h">Name</span><input aria-labelledby="h"></main>`)),
    ).toEqual([]);
  });

  test("an unlabelled input fails, and names the rule", () => {
    const findings = checkHtml(page(`<main><input id="n"></main>`));
    expect(findings.map((f) => f.rule)).toEqual(["control-unlabelled"]);
    expect(findings[0]!.message).toContain("<input>");
  });

  test("a placeholder is not a name", () => {
    // The exact trap: it looks labelled on screen and is announced as "edit text".
    const findings = checkHtml(page(`<main><input placeholder="Name"></main>`));
    expect(findings.map((f) => f.rule)).toEqual(["control-unlabelled"]);
  });

  test("hidden and button-ish inputs are exempt", () => {
    expect(
      checkHtml(page(`<main><input type="hidden" name="x"><input type="submit"></main>`)),
    ).toEqual([]);
  });

  test("a select and a textarea are held to the same rule", () => {
    expect(checkHtml(page(`<main><select></select></main>`)).map((f) => f.rule)).toEqual([
      "control-unlabelled",
    ]);
    expect(checkHtml(page(`<main><textarea></textarea></main>`)).map((f) => f.rule)).toEqual([
      "control-unlabelled",
    ]);
  });

  test("the line number points at the control, not the document", () => {
    const html = `<!doctype html>
<html>
<body>
<main>
<form>
<input id="n">
</form>
</main>
</body>
</html>`;
    expect(checkHtml(html)[0]!.line).toBe(6);
  });
});

describe("a11y: headings", () => {
  test("h1 then h2 passes", () => {
    expect(checkHtml(page(`<main><h1>A</h1><h2>B</h2></main>`))).toEqual([]);
  });

  test("a skipped level fails", () => {
    const findings = checkHtml(page(`<main><h1>A</h1><h3>C</h3></main>`));
    expect(findings.map((f) => f.rule)).toEqual(["heading-skipped"]);
    expect(findings[0]!.message).toContain("<h3> follows <h1>");
  });

  test("starting at h2 is allowed", () => {
    // Not ideal, but not a skip: a page whose title is in a native chrome has no
    // h1 in the document, and failing that would be wrong.
    expect(checkHtml(page(`<main><h2>B</h2></main>`))).toEqual([]);
  });
});

describe("a11y: landmarks", () => {
  test("main is a landmark", () => {
    expect(checkHtml(page(`<main><p>x</p></main>`))).toEqual([]);
  });

  test("role=main is the same landmark", () => {
    expect(checkHtml(page(`<div role="main"><p>x</p></div>`))).toEqual([]);
  });

  test("no landmark fails", () => {
    expect(checkHtml(page(`<p>x</p>`)).map((f) => f.rule)).toEqual(["no-landmark"]);
  });
});

describe("a11y: the scaffold is clean", () => {
  test("the shipped template passes every rule", async () => {
    const { readFileSync } = await import("node:fs");
    const { join } = await import("node:path");
    const template = readFileSync(
      join(import.meta.dir, "..", "template", "index.html.tmpl"),
      "utf8",
    );
    // The template carries text that is not HTML (`{{NAME}}` in attributes), which
    // no rule depends on: the checks read tags, not values.
    expect(checkHtml(template)).toEqual([]);
  });
});
