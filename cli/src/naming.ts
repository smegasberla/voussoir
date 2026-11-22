/** Derive every identifier an app needs from one name.
 *
 *  Done in one place because these are easy to get subtly wrong, and a wrong
 *  package name produces a JNI symbol mismatch that shows up as
 *  `UnsatisfiedLinkError` at launch rather than as a build error.
 */

export type Identifiers = {
  /** Human-facing name, e.g. "My Notes". */
  display: string;
  /** Crate name and root directory slug, e.g. "my-notes". */
  crate: string;
  /** Rust `lib` name for the Android cdylib, e.g. "my_notes". Must be a valid identifier. */
  lib: string;
  /** Android package, e.g. "com.voussoir.my_notes". */
  package: string;
  /** JNI mangling of the package, e.g. "com_voussoir_my_1notes". */
  jniPackage: string;
  /** First segment of the JNI package for tao, e.g. "com". */
  domain: string;
  /** Last segment of the JNI package for tao, e.g. "my_notes". */
  leaf: string;
};

/** Slug for directories and crates: lowercase, hyphens, no leading digits. */
export function crateName(name: string): string {
  const base = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  const safe = base === "" ? "voussoir-app" : base;
  return /^[0-9]/.test(safe) ? `app-${safe}` : safe;
}

/** Rust identifier: underscores, and never a keyword. */
export function libName(crate: string): string {
  const snake = crate.replace(/-/g, "_");
  const keywords = new Set([
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
  ]);
  return keywords.has(snake) ? `${snake}_app` : snake;
}

/** Java keywords, which cannot be a package segment. */
const JAVA_KEYWORDS = new Set([
  "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char",
  "class", "const", "continue", "default", "do", "double", "else", "enum",
  "extends", "final", "finally", "float", "for", "goto", "if", "implements",
  "import", "instanceof", "int", "interface", "long", "native", "new",
  "package", "private", "protected", "public", "return", "short", "static",
  "strictfp", "super", "switch", "synchronized", "this", "throw", "throws",
  "transient", "try", "void", "volatile", "while", "true", "false", "null",
]);

/** Refuse a package that Gradle, javac and the JNI mangling cannot all accept.
 *
 *  A package reaches the Gradle `namespace`, the generated Java directory and the
 *  JNI symbol names, so a bad one used to be discovered as an
 *  `UnsatisfiedLinkError` at launch instead of as an `init` error. Lowercase is
 *  required because Android tooling rejects mixed case in a namespace, and `_`
 *  is allowed because the default is derived from the crate name.
 */
export function validatePackage(pkg: string): void {
  const segments = pkg.split(".");
  if (segments.length < 2) {
    throw new Error(
      `\`${pkg}\` is not a package name; use at least two dot-separated segments, like com.example.notes`,
    );
  }
  for (const segment of segments) {
    if (!/^[a-z][a-z0-9_]*$/.test(segment)) {
      throw new Error(
        `\`${pkg}\` is not a valid package: each segment must start with a lowercase letter and contain only lowercase letters, digits and underscores (offending segment: \`${segment}\`)`,
      );
    }
    if (JAVA_KEYWORDS.has(segment)) {
      throw new Error(`\`${pkg}\` is not a valid package: \`${segment}\` is a Java keyword`);
    }
  }
}

/** A reverse-DNS package. `com.voussoir.<slug>` by default, overridable by the user. */
export function packageName(crate: string, override?: string): string {
  if (override) {
    validatePackage(override);
    return override;
  }
  return `com.voussoir.${libName(crate)}`;
}

/** JNI mangles `_` as `_1` and `.` as `_`. */
export function jniPackage(pkg: string): string {
  return pkg.replace(/_/g, "_1").replace(/\./g, "_");
}

export function identifiers(name: string, packageOverride?: string): Identifiers {
  const crate = crateName(name);
  const lib = libName(crate);
  const pkg = packageName(crate, packageOverride);
  const segments = pkg.split(".");
  return {
    display: name.trim() === "" ? "Voussoir App" : name.trim(),
    crate,
    lib,
    package: pkg,
    jniPackage: jniPackage(pkg),
    domain: segments.slice(0, -1).join("_") || "com",
    leaf: segments[segments.length - 1] ?? lib,
  };
}
