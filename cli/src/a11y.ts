/** Static accessibility checks for a scaffolded page.
 *
 *  The webview builds the OS accessibility tree from the markup, so a page is
 *  navigable exactly as far as its roles, labels and heading order go. These are
 *  the three defects that actually ship in hand-written frontends, checked on the
 *  file rather than guessed at runtime:
 *
 *  - a form control with no accessible name (a reader says "edit text"),
 *  - a heading level that skips (the heading list becomes a lie),
 *  - no landmark, so there is nothing to jump to.
 *
 *  Deliberately a *file* check and not a runtime one. A runtime tree would have to
 *  be built by us and would then disagree with the DOM; this reads the same source
 *  the webview reads, and it costs milliseconds in `voussoir doctor`.
 */

import { readFileSync } from "node:fs";

export type A11yFinding = {
  /** Stable id, so a test asserts the rule and not the wording. */
  rule: "control-unlabelled" | "heading-skipped" | "no-landmark";
  message: string;
  line: number;
  /** The offending text, trimmed, for the report. */
  source: string;
};

/** Check one HTML document. `undefined` when the file could not be read. */
export function checkHtml(html: string): A11yFinding[] {
  const findings: A11yFinding[] = [];
  const lines = html.split("\n");

  const at = (index: number) => {
    const before = html.slice(0, index).split("\n").length - 1;
    return { line: before + 1, source: (lines[before] ?? "").trim().slice(0, 120) };
  };

  // 1. Controls need a name: a <label for=...>, an aria-label, or aria-labelledby.
  //    A `placeholder` is not a name — it disappears the moment the user types.
  const labelFor = new Set<string>();
  for (const match of html.matchAll(/<label\b[^>]*\bfor\s*=\s*["']([^"']+)["']/gi)) {
    labelFor.add(match[1]!);
  }
  const labelledIds = new Set<string>();
  for (const match of html.matchAll(/<label\b[^>]*>[\s\S]*?<(\w+)\b[^>]*\bid\s*=\s*["']([^"']+)["']/gi)) {
    labelledIds.add(match[2]!);
  }
  // The span of every `<label>…</label>` in the document, so a control can be
  // asked "are you inside one?" rather than grepped against a fixed window. A
  // window was wrong for the obvious case: `<label>Name <input></label>` puts the
  // control *at* the edge of any slice taken before it.
  const labelSpans: [number, number][] = [];
  for (const match of html.matchAll(/<label\b[^>]*>([\s\S]*?)<\/label>/gi)) {
    labelSpans.push([match.index!, match.index! + match[0].length]);
  }
  const insideLabel = (index: number) =>
    labelSpans.some(([start, end]) => index > start && index < end);

  for (const match of html.matchAll(/<(input|select|textarea)\b([^>]*)>/gi)) {
    const tag = match[1]!.toLowerCase();
    const attrs = match[2] ?? "";
    if (/\btype\s*=\s*["'](hidden|submit|button|reset|image)["']/i.test(attrs)) continue;
    if (/\baria-label(ledby)?\s*=/i.test(attrs)) continue;
    const id = /\bid\s*=\s*["']([^"']+)["']/i.exec(attrs)?.[1];
    if (id && (labelFor.has(id) || labelledIds.has(id))) continue;
    if (insideLabel(match.index)) continue;
    const where = at(match.index);
    findings.push({
      rule: "control-unlabelled",
      message: `<${tag}> has no label: add <label for="…">, or aria-label`,
      line: where.line,
      source: where.source,
    });
  }

  // 2. Heading levels must descend one at a time. `<h1>` then `<h3>` gives a
  //    reader a list with a hole in it.
  let previous = 0;
  for (const match of html.matchAll(/<h([1-6])\b/gi)) {
    const level = Number(match[1]);
    if (previous !== 0 && level > previous + 1) {
      const where = at(match.index);
      findings.push({
        rule: "heading-skipped",
        message: `<h${level}> follows <h${previous}>: heading levels must not skip`,
        line: where.line,
        source: where.source,
      });
    }
    previous = level;
  }

  // 3. One landmark, so there is somewhere to jump to. `<main>` is the one a
  //    reader lists first; `role="main"` on an element is the same thing.
  if (!/<main\b/i.test(html) && !/role\s*=\s*["']main["']/i.test(html)) {
    findings.push({
      rule: "no-landmark",
      message: "no <main> landmark: a reader has nothing to jump to",
      line: 1,
      source: (lines[0] ?? "").trim(),
    });
  }

  return findings;
}

/** Check the file at `path`, or `[]` when it does not exist. */
export function checkFile(path: string): A11yFinding[] {
  try {
    return checkHtml(readFileSync(path, "utf8"));
  } catch {
    return [];
  }
}
