# Roadmap: from a language one person writes to a language GitHub names

GitHub's language bar comes from [github-linguist/linguist](https://github.com/github-linguist/linguist).
Getting `Deed` to appear there is the milestone this document is written around,
but it is not the goal. It is a **lagging indicator**: Linguist's entry
requirement is a measurement of how many people use a language, so the entry
arrives when the adoption is real and cannot be made to arrive before that.

Everything below is measured. Where a number appears, the command that produced
it is next to it, so this document can be re-run rather than believed.

---

## 1. The requirement, in Linguist's own words

From [CONTRIBUTING.md](https://github.com/github-linguist/linguist/blob/main/CONTRIBUTING.md):

> The usage requirements are:
>
> - at least **2000 files per extension** or filename indexed in the last year
>   (the number you see at the top of the search results), **excluding forks**,
>   for extensions or filenames expected to occur more than once per repo […]
> - the results should show a **reasonable distribution across unique
>   `:user/:repo` combinations** […] If particular users are showing a high
>   proportion of the results, for example **the primary language owner**, we
>   will filter out those users using `-user:<username>`.

And, so no time is wasted on the shortcut:

> we do not accept PRs for very new or hobby languages, and **will close any
> such PRs** that attempt to add them.

The `.gitattributes` escape hatch does not exist either. From
[overrides.md](https://github.com/github-linguist/linguist/blob/main/docs/overrides.md):

> Languages that are not yet mentioned in `languages.yml` **will not be included
> in the language statistics**, even if you specify something like
> `*.mycola linguist-language=MyCoolLang linguist-detectable` in the
> `.gitattributes` file.

So there is exactly one path, and it runs through other people's repositories.

---

## 2. Where we are, measured on 2026-08-12

| Thing | Measured | How |
| --- | --- | --- |
| `.deed` files on GitHub, all repos | **87** | code search `extension:deed` |
| Files with the extension outside `deed-lang` | 6 | `extension:deed -user:deed-lang` |
| …of those that are **actually Deed source** | **0** | see §2a |
| Repositories containing Deed source | **1** | same |
| Stars | **7** | `gh api repos/deed-lang/deed` |
| Forks | **0** | same |
| Release downloads, all 12 releases | **26** | `gh api …/releases` |
| Repo age | **18 days** | created 2026-07-25 |

The distance is not 2000 − 87. It is 2000 − 0, because the owner is filtered
out. At a typical ~10 `.deed` files per project, the shape of the target is:

> **≈ 200 public repositories, owned by ≈ 200 different people, containing
> ≈ 2000 `.deed` files between them.**

That is the whole problem stated honestly. Nothing in this document makes it
smaller; the stages below only make it reachable in order.

### 2a. `.deed` is a contested extension, and counting it naively lies

Searching `extension:deed -user:deed-lang` returns **6**, which looks like
progress and is not. All six are in `SNAPKITTY-COLLECTIVE-LIMITED-FLP/agentic-arena`
and `SNAPKITTYWEST/bob-ide`, and they are not Deed:

```
(agent-deed
  (id         "BOB")
  (role       oracle)
  (authority
    (may      read)
    (may-not  delete)))
```

S-expressions describing agent permissions. Somebody else reached for the same
word and the same extension, which is worth knowing for two reasons:

- **The metric needs a Deed-specific token in it**, or the number counts
  strangers.
- **Linguist has a rule for this.** If an extension is already claimed, the PR
  needs two samples per language for that extension and, where the languages
  could be confused, a heuristic. These two cannot be confused — Deed files open
  with `module`, those open with `(` — so a heuristic is writable. It is a
  paperwork item to remember at Stage 3, not a threat.

### Measure it again

```powershell
# The only number that matters: files that are Deed, in repos we do not own.
gh api -X GET search/code --raw-field q='extension:deed "ensures" -user:deed-lang' --jq '.total_count'
# 2026-08-12: 0

# The control. If this one is also 0, the query broke — it is not that Deed
# vanished. A metric that cannot tell "nothing there" from "nothing measured"
# is worse than no metric.
gh api -X GET search/code --raw-field q='extension:deed "ensures"' --jq '.total_count'
# 2026-08-12: 8

# Distribution: how many distinct owners, not just how many files.
gh api -X GET search/code --raw-field q='extension:deed "ensures" -user:deed-lang' --paginate `
  --jq '.items[].repository.full_name' | ForEach-Object { $_.Split('/')[0] } | Sort-Object -Unique
```

GitHub code search only indexes what it indexes; treat these as a floor. The
`"ensures"` token undercounts too — a Deed file with no contract in it will not
match — which is the right direction for a number used to decide whether to
open a PR.

---

## 3. What is already done

These are the parts of a Linguist PR that are in our control, and they are
finished. Nothing here needs work; it is listed so it is not redone.

- **TextMate grammar.** `editors/vscode/syntaxes/deed.tmLanguage.json`,
  `"name": "Deed"`, `"scopeName": "source.deed"`. Linguist highlights with
  TextMate grammars, not tree-sitter, so this is the right artifact and it
  already exists.
- **A permissive licence.** Apache-2.0, which is on Linguist's accepted list.
- **Real samples.** `std/*.deed` and `examples/*.deed` are working programs, not
  tutorials. Linguist rejects "hello world and other examples found in
  tutorials"; `std/list.deed` and `examples/transfer.deed` are neither.
- **A tree-sitter grammar** (`editors/tree-sitter-deed/`) for editors that want
  one. Not required by Linguist, but it is what the wider ecosystem asks for.

Calibration: **Hare is in `languages.yml` with `tm_scope: none`** — no grammar at
all. Gleam, Roc, Koka, Jai, Carbon, MoonBit and Quint are all in as well. Young
languages do get accepted. What they had that we do not is users.

**Conclusion: the grammar is not the blocker and never was. Adoption is.**

---

## 4. Blockers in our control, measured today

These were found by looking rather than guessed. Each one sits directly on the
path between "somebody hears about Deed" and "a public repo contains `.deed`
files", which is the only path that moves the number in §2.

### 4a. There is no way to start a project — **closed**

`deed --help` used to list: `check`, `test`, `run`, `build`, `doc`, `fmt`,
`fix`, `explain`, `lsp`, `debug`, `mcp`. There was no `deed new` and no `deed
init`, so someone who decided to try Deed had to work out the module header,
the file layout and the manifest by reading this repository. **The metric
Linguist counts is repositories containing several `.deed` files, and nothing
in the toolchain created one.**

`deed new <name>` now writes a directory holding a library module with a
contract and its tests, and a program that imports it — two `.deed` files, and
no manifest, because a manifest here names code outside your tree and a new
project has none. `crates/deed-cli/tests/new.rs` runs it into a temporary
directory on every commit and then checks, tests, runs and format-checks the
result, so the scaffold cannot rot into something that does not compile.

### 4b. Installing requires cloning the repo and having Rust — **closed**

README used to say `cargo install --path crates/deed-cli` and nothing else.
There was no `curl | sh`, no Homebrew formula, no winget package, and no
`cargo install deed`. Twenty-six downloads across twelve releases is what that
cost.

`install.sh` and `install.ps1` now fetch the release asset for the machine,
refuse it against the release's own checksum list, and install one file into
the user's profile without asking for a password. Homebrew and winget are
still absent, and both of them want a stable install base first.

### 4c. The crates.io name is taken, and the fallback is claimed — **closed**

- `deed` on crates.io: owned by **`degenie-ai`**, version **0.0.0**, published
  **2026-07-06** — three weeks before this repository existed. A placeholder.
  `cargo install deed` still installs somebody else's crate.
- **`deed-lang` is ours**, along with the nineteen crates the compiler is made
      of, all at 0.2.10 (2026-08-12). `cargo install deed-lang` installs a `deed`.

Getting there took four days' worth of defects out of one afternoon, all found
with `cargo package --list` and `cargo publish --workspace --dry-run` rather
than assumed:

- [x] `deed-explain` would have published **successfully and empty**. Its pages
      came from a build script that read the whole workspace, and a `.crate`
      carries one package directory. Every `deed explain` would have printed
      nothing. Fixed: the pages are generated, committed and shipped as source.
- [x] `deed-driver` would not compile at all. `src/shipped.rs` embedded the
      nine `std/*.deed` modules with `include_str!("../../../std/...")`, and
      `cargo package -p deed-driver --list` carried none of them. Fixed: the
      text is generated into the crate, and a rule now holds every package
      against reading above its own root.
- [x] Path dependencies carried no version, which `cargo publish` refuses.
      Fixed: declared once in `[workspace.dependencies]`, held against
      `workspace.package.version` by a test.
- [x] Nothing ran `cargo publish --workspace --dry-run`. CI runs it now, as
      "it could be published".

The install was verified the way a stranger does it: `cargo install deed-lang`
into an empty root, then `deed explain DEED4025`, `deed new`, `deed test` and a
program that imports `std/list`. The first two are exactly what the two bugs
above would have broken.

⚠️ crates.io allows five new crates and then one every ten minutes, so twenty
crates take about two and a half hours. `cargo publish --workspace` cannot
resume: a crate already on the index is a warning under `--dry-run` and an
error on the real run. Publish them one at a time, in dependency order.

**The two install paths are on the same version.** The release and all twenty
crates.io packages are at 0.2.10 as of 2026-08-12. This was verified through the
registry API and by installing `deed-lang` into an empty root, then running
`deed --version`, `deed new` and the generated project's three tests.

```powershell
# Are they the same today?
(Invoke-RestMethod "https://crates.io/api/v1/crates/deed-lang").crate.max_version
gh release view --json tagName --jq .tagName
```

### 4d. There is no way to depend on somebody else's code

`how-to/depend-on-another-module.md` is written and
`design/decisions/2026-08-07-a-dependency-is-a-location-and-a-hash.md` explains
why there is no registry. Both are honest. The consequence is still that a
person cannot publish a Deed library and a second person cannot find it, and a
language where nobody can build on anybody produces very few repositories.

### 4e. Known compiler gaps, already recorded

Carried here so the roadmap does not pretend they are closed:

- No value reclamation. A compiled program gives nothing back except a handler
  frame, so total allocation is peak memory. The measured shape of what is left
  is one fact and it is interprocedural: the same `push` moved into a two-line
  function allocates the answer once per element again, sixty-five times over at
  a length of a hundred and twenty-eight. See
  `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md`.

  The old entry here said `examples/logs.deed` exhausts memory at a 4 GB
  ceiling and named this page as the reason. That was wrong twice over: the
  program died because `str_concat` moved the bump pointer without growing the
  memory, and it now runs compiled and prints the same 69,680 bytes the
  interpreter prints. A trap whose message names a resource is not a
  measurement of that resource.

- A record or a choice crossing a component boundary, or a list of anything but
  numbers. `deed build --component` writes a component binary, and numbers,
  booleans, text and `list<s64>` cross it; anything wider is refused by name,
  because its lowering is per element and per field. See
  `design/decisions/2026-08-09-text-crosses-the-component-boundary.md`.

- No dependency discovery, which is §4d and is the one on this list that costs
  repositories rather than programs.

---

## 5. Stages

Gates, not dates. A stage is finished when its exit condition is **measured**,
and the next stage does not start before that. Every stage's exit condition is a
number that can be checked with the commands in §2.

### Stage 0 — Stop the leaks

Exit: a person who wants to try Deed can install it and produce a working
project without reading the compiler's source.

- [x] Claim **`deed-lang`** on crates.io. Do this first; it is the only item
      here that can be lost by waiting. *Done 2026-08-09: `deed-lang` and the
      nineteen crates the compiler is made of; all were published at 0.2.10 on
      2026-08-12.*
- [x] `deed new <name>` — scaffolds a module, a test, and whatever manifest
      `how-to/depend-on-another-module.md` specifies. Held by a driver test that
      runs `deed new` into a temp directory and then runs `deed test` on the
      result, so the scaffold cannot rot into something that does not check.
      *Done. It writes two modules and no manifest, and the reason for the
      second of those is in §4a.*
- [x] One-line install that does not need Rust: a script that fetches the
      release binary for the platform. The four release artifacts already exist;
      nothing points at them. *Done: `install.sh` and `install.ps1`, and the
      release now publishes a checksum list so they can refuse a download.*
- [x] README's first code block becomes install → `deed new` → `deed test`,
      in that order.

### Stage 1 — Make the first non-author repository exist

Exit: **`extension:deed "ensures" -user:deed-lang` returns more than 0**, with
the control query in §2 returning non-zero on the same day.

This is the largest single step in the whole document, because it is the step
from zero. Everything after it is multiplication; this one is addition.

- [x] Publish the benchmark result as its own artefact. Five runs of one model
      against one build, with the DEED2003/DEED3001 table, is a measurement
      almost no language has made about itself. It is the most defensible
      interesting thing this project owns. *Done: [benchmarks/RESULTS.md](benchmarks/RESULTS.md).
      Writing it found a real defect — the scorer was reporting proven
      obligations for answers the compiler had rejected, so the control arm's
      row read `0 check` with `proven 1` beside it.*
- [x] Write the `deed mcp` story down where agent developers read it. "A
      compiler an agent can ask questions of, and a benchmark that measures
      whether the agent got it right" is a sentence no other language can
      currently say. *Done: <https://deed-lang.github.io/agents/>. It had been
      one paragraph on the install page, which is not where somebody deciding
      whether to wire a compiler into an agent is reading. Every transcript on
      it is filled in by the pinned compiler in the reader's tab — an
      obligation carrying a tier, a `guarded` one with its reason, the `export`
      repair, and a property generated from a contract in a module with no
      tests — so the page cannot claim an answer the compiler does not give.*
- [ ] Ship a second model family in the benchmark. Today only `OPENAI_API_KEY`
      is configured, so "does this hold across model families" is unmeasured,
      and the result is quotable only with that caveat attached.
- [x] The comparison arm: the same six behaviours in contracts-free Starlark.
      One no-tool run passed 6/6 where the five Deed MCP runs passed 5/6. This
      does not establish an effect size, but it does falsify the claim that the
      current six tasks show contracts helping. The exact answers and the next
      benchmark requirement are in `benchmarks/STARLARK.md`.

### Stage 2 — Make the second repository not need us

Exit: **≥ 25 distinct owners** with `.deed` files, and at least one repository
we did not write and were not asked about.

- [ ] Dependency discovery. §4d, in whatever the smallest form is — an index
      file in this repo listing known modules is enough to start and requires no
      registry.
- [x] Value reclamation, or an honest ceiling in the docs. People stop using a
      language the first time it dies on their real input. *The second half is
      done: `how-to/embed-a-compiled-program.md` now says that nothing is given
      back, what that costs across repeated calls, and that a fresh instance is
      what returns it. The numbers on the page are produced by
      `crates/deed-codegen/smoke.mjs` in a real engine on every commit and
      compared against the page, so it cannot quote a compiler two releases old.
      Reclamation itself is still open and now has a named next step in
      `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md`.*
- [ ] `deed new --lib` and a worked example of one person's module being used by
      another person's program.

### Stage 3 — Cross the line

Exit: **≥ 2000 `.deed` files, ≥ 200 distinct owners**, sustained over a year of
GitHub's index.

- [ ] Open the Linguist PR. By this point it is paperwork: the grammar, licence
      and samples from §3 have been ready since the beginning. Follow
      CONTRIBUTING exactly, use the template, link the search excluding
      `deed-lang`, and state the samples' licence.
- [ ] Handle the shared extension from §2a: two samples for each language using
      `.deed`, and a heuristic if the other use is in `languages.yml` by then.
      `module` versus a leading `(` makes that one line.
- [ ] Pick the colour before the PR, not during it. Linguist asks for community
      consensus on the colour in a forum the language's users read, and links to
      that discussion in the PR.

---

## 6. What not to do

Linguist's assessment explicitly includes **manually and randomly clicking
through the results** and filtering out users who account for a high proportion
of them. Manufacturing the number would therefore fail on its own terms, and
would be the one kind of failure this project could not write an honest
changelog entry about.

Concretely, do not:

- create repositories to hold `.deed` files,
- ask people to commit generated Deed,
- map `.deed` to another language in `.gitattributes` so the bar shows
  something. The bar would say "Rust", not "Deed", and it would be a lie about
  the repository's contents.

The measurement in §2 exists so we can tell whether the number moved for a
reason. A number that moved for no reason is worse than a small one.

---

## 7. What would falsify this plan

In the spirit of `benchmarks/README.md`, which asks the same question of the
benchmark:

If Stage 1 does not complete — if, after the install story works and the
benchmark result is published, **still nobody outside this repository writes a
`.deed` file** — then the problem is not distribution and this roadmap is the
wrong document. It would mean the language does not yet solve a problem somebody
has, and the correct response is to find that problem, not to keep improving the
funnel into it.

That is a real possible outcome. Stage 1's exit condition is deliberately the
smallest number in this file — *more than zero* — because it is the one that
tests the premise rather than the execution.

---

## 8. Today's honest summary

- Everything Linguist needs from **us** is done.
- Everything Linguist needs from **the world** is at zero. Not near zero: the
  six files that carry the extension elsewhere are somebody else's format.
- Stage 1 is still open on 2026-08-12: the Deed-specific outside-owner query is
      **0** while its control is **8**.
- The gap is not technical, and no amount of compiler work closes it.
- Stage 0 is closed. Somebody who hears about Deed can install it in one line,
  or with `cargo install deed-lang`, and `deed new` gives them a project that
      checks. Nothing on that path now requires reading this repository, and both
      install paths deliver 0.2.10 (§4c).
- The next action is the one nobody else can do for us: **give somebody outside
  this repository a reason to write a `.deed` file**, and the two remaining
  Stage 1 items with no clock on them are the measurements that would make the
  reason checkable rather than assertable.
