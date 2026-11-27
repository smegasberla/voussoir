/** Minimal argv parsing: `voussoir <command> [flags] [positional]`. */

export type Args = {
  command: string;
  positionals: string[];
  flags: Record<string, string | true>;
  /**
   * Every value a repeatable flag was given, in order.
   *
   * `flags` keeps only the last one, which is the right default (a second
   * `--out` should win) but loses information for `--previous`, where each
   * repetition is a different previous artifact to patch from.
   */
  repeated?: Record<string, string[]>;
  help: boolean;
};

export function parseArgs(argv: string[]): Args {
  const leadingFlags = argv[0]?.startsWith("-") ?? false;
  const [command = "help", ...rest] = leadingFlags ? ["help", ...argv] : argv;
  const positionals: string[] = [];
  const flags: Record<string, string | true> = {};
  const repeated: Record<string, string[]> = {};
  let help = false;

  const record = (key: string, value: string) => {
    flags[key] = value;
    (repeated[key] ??= []).push(value);
  };

  for (let i = 0; i < rest.length; i++) {
    const token = rest[i]!;
    if (token === "-h" || token === "--help") {
      help = true;
    } else if (token.startsWith("--")) {
      const [rawKey, inline] = token.slice(2).split("=", 2);
      const key = rawKey ?? "";
      if (inline !== undefined) {
        record(key, inline);
      } else {
        const next = rest[i + 1];
        if (next !== undefined && !next.startsWith("-")) {
          record(key, next);
          i++;
        } else {
          flags[key] = true;
        }
      }
    } else {
      positionals.push(token);
    }
  }

  return { command, positionals, flags, repeated, help };
}

export function stringFlag(args: Args, name: string): string | undefined {
  const value = args.flags[name];
  return typeof value === "string" ? value : undefined;
}

/** Every value given to a repeatable flag, in order; a single flag still lists one. */
export function listFlag(args: Args, name: string): string[] {
  return args.repeated?.[name] ?? [];
}

export function boolFlag(args: Args, name: string): boolean {
  return args.flags[name] === true || args.flags[name] === "true";
}
