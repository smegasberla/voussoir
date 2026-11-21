# Writing docs

The documentation is plain Markdown in `docs/src`, validated by
`voussoir docs --check` and rendered into the website by the same command. There is
no second renderer and no second copy of the text.

## Theory: why plain Markdown and one source

Most projects end up with two documentation sets that drift: one in the repository
for people reading code, one on a site for people who need it. The drift is not
laziness, it is the cost of two formats.

Keeping one set of Markdown files, and rendering them, makes drift impossible
rather than unlikely. GitHub shows the same text the site does, so a reader who
finds a page through a code search sees what the site says.

The other decision is the renderer: `Bun.markdown`, which is already a dependency of
the build, instead of a documentation generator with its own config and theme. A
generator would add a tool to install and a theme to fight; the site's CSS is
ordinary CSS and the generator is one file.

## The structure

```
docs/src/
  SUMMARY.md      the table of contents, and the navigation order
  *.md            one chapter per file
```

`SUMMARY.md` is an ordinary Markdown list with indentation for nesting:

```markdown
- [Concepts](concepts.md)
- [Installing Voussoir](install.md)
- [Your first app](first-app.md)
  - [How a project is laid out](layout.md)
  - [The frontend](frontend.md)
```

Indentation is read as **presence or absence**, not as a count: the renderer takes
any leading whitespace to mean "child of the nearest unindented entry above". So a
sub-item is a sub-item whether it is indented two spaces or six, and there is no
third level, because a third level would look identical to a second one.

**Why a list and not front matter:** because the file that drives navigation is then
a file a person can read and reorder, and the check can validate it with the same
parser that renders it.

## What `--check` enforces

```bash
voussoir docs --check
```

Three rules, and each corresponds to a way a docs folder rots:

1. **Every entry in `SUMMARY.md` exists.** A renamed file leaves a dangling entry,
   and a sidebar link to a 404 is worse than no link.
2. **Every `.md` file appears in `SUMMARY.md`.** A new chapter that was never linked
   is invisible; it costs one line to catch.
3. **No relative link points at a chapter the summary does not list.** This is the
   interesting one, and it is free in a renderer that fails on a missing page.

Rule 3 exists because links between chapters are the most common way a docs set
breaks: split a big page in two, forget one reference, and the reader lands on a
page that was never generated.

**What it does not check:** prose, spelling, or whether a claim is true. Those are
review, and the check exists so review spends its attention on them rather than on
dangling filenames.

## Adding a chapter

1. Write `docs/src/your-topic.md`, starting with a single `# Title`.
2. Add it to `SUMMARY.md` at the depth you want.
3. `voussoir docs --check`; fix what it reports.
4. `voussoir docs` to see the rendered page.
5. Link it from wherever a reader would arrive from. An unlinked chapter is a
   chapter nobody reads.

## Theory: the shape of a chapter

Every chapter in this book uses the same structure, and it is not decoration:

| Part | Purpose |
| --- | --- |
| What the thing is | one paragraph, no jargon the glossary does not define |
| `## Theory:` block | why it exists, the trade it makes, what goes wrong without it |
| Practice | the code, the flags, the file paths, in order of use |
| "What it does not do" | the limits, so a reader can stop looking |
| `## Next` | where to read after this |

The theory block is the part most documentation skips and most readers need. A
reference tells you what the current behaviour is; it does not tell you which part
is a deliberate trade, so you cannot predict the case the table omits.

The "what it does not do" section is the second most valuable. A limit stated once
saves every reader from discovering it in production.

## Theory: writing code samples that stay true

A code sample is a claim that this compiles and does what the paragraph says. Two
habits keep samples from lying:

- **Copy from working code.** If a sample cannot be lifted from the repository, it is
  written from memory, and memory is where the wrong method name comes from.
- **Keep samples small enough to run.** A twenty-line sample that includes setup,
  the interesting call, and nothing else can be checked by hand; a sixty-line one
  cannot.

When a sample and the source disagree, the source wins and the sample is a bug. This
book has had three such bugs found and fixed, which is a reason to keep checking
rather than a reason to stop.

## What the site adds

The renderer adds chrome only: navigation, the stylesheet, and the arch on the home
page. The chapter text is passed through, so a construct that works on GitHub works
on the site, with two exceptions:

- **Tables** need the pipe syntax. HTML tables render but lose the styling.
- **Footnotes** are not supported by `Bun.markdown`; use a list.

## Next

- [Command-line reference](cli.md) — the `docs` command in context.
- [Concepts](concepts.md) — the chapter most new readers start with.
