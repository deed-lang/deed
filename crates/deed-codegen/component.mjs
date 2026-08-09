// What a real component toolchain makes of what `deed build --component` wrote.
//
// The claim `deed build --component` is for is `design/05-backend.md`'s: a
// program's effect row is its world, and nothing else derives one. For four
// releases the world was written as `.wit` text and nothing in this repository
// read it back, which is the shape of a claim nobody who would have to believe
// it had ever tested. This file was written to ask the toolchain that would,
// and what it measured was a gap: `wasm-tools component new` turned the core
// module into a component that exported nothing.
//
// That gap is closed, so this file now measures the thing working rather than
// the thing missing. Three halves of it, because the third is still true:
//
//   - a component `deed build --component` wrote whose exports need no
//     adapters, which answers correctly when a component runtime calls it;
//   - one whose exports carry text, which needs `cabi_realloc` and the two
//     halves of a string, and which answers correctly through the same
//     runtime;
//   - and a module carrying a list, which is not given a component at all,
//     because the adapters for that are not written.
//
// `jco` bundles the Bytecode Alliance's own `wasm-tools` and adds a
// transpiler, so the calls below go through a real component runtime rather
// than through anything in this workspace.
//
//   node crates/deed-codegen/component.mjs <path to deed> <scratch directory>

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [deed, scratch] = process.argv.slice(2);
if (!deed || !scratch) {
  console.error("usage: component.mjs <path to deed> <scratch directory>");
  process.exit(2);
}

let failures = 0;
function check(what, ok, detail) {
  if (ok) {
    console.log(`ok    ${what}`);
  } else {
    failures += 1;
    console.log(`FAIL  ${what}${detail === undefined ? "" : `: ${detail}`}`);
  }
}

// Through a shell, because npm installs it as a wrapper script and Node will
// not spawn one directly on Windows.
const jco = (args, options = {}) => execFileSync("jco", args, { shell: true, ...options });

mkdirSync(scratch, { recursive: true });

// -- what crosses unchanged -------------------------------------------------
//
// No `main` and no capability in a signature, which is what a component is
// allowed to be, and nothing wider than a word in either direction, which is
// what can be lifted without adapters. Two types rather than one: a number and
// a boolean are different bytes on the boundary, and swapping them is a
// component that lies about what it takes.
const FLAT = `module adder

fn add(a: Int, b: Int) -> Int { a + b }

fn positive(n: Int) -> Bool { n > 0 }
`;

const flat = join(scratch, "adder.deed");
writeFileSync(flat, FLAT);
execFileSync(deed, ["build", "--component", flat], { stdio: "pipe" });

const world = readFileSync(join(scratch, "adder.wit"), "utf8");
check(
  "the world names both functions the module declares",
  world.includes("export add:") && world.includes("export positive:"),
  JSON.stringify(world),
);

// The core module is still a core module. A host embedding one reads this
// file, and it is the same bytes `deed build` writes.
const core = readFileSync(join(scratch, "adder.wasm"));
const module = await WebAssembly.compile(core);
const exports = WebAssembly.Module.exports(module).map((one) => one.name);
check(
  "and the core module beside it still exports them",
  exports.includes("add") && exports.includes("positive"),
  JSON.stringify(exports),
);

// The component, read with the tooling somebody adopting the component model
// would read it with. This is the assertion that used to say the world was
// empty.
const component = join(scratch, "adder.component.wasm");
const produced = jco(["wit", component], { encoding: "utf8" });
check(
  "and the component it wrote has a world with both of them in it",
  produced.includes("export add:") && produced.includes("export positive:"),
  produced.trim(),
);

// Reading a world is not running one. Anything that got the lift wrong -- the
// wrong core function, the wrong type, a parameter in the wrong place -- still
// produces a component whose world reads correctly.
jco(["transpile", component, "-o", join(scratch, "js"), "--name", "adder"], { stdio: "pipe" });
const running = await import(pathToFileURL(resolve(scratch, "js", "adder.js")).href);
check(
  "a component runtime runs it",
  running.add(20n, 22n) === 42n,
  `add(20, 22) = ${running.add(20n, 22n)}`,
);
check(
  "and the second export is the second function rather than the first",
  running.positive(3n) === true && running.positive(-1n) === false,
  `positive(3) = ${running.positive(3n)}, positive(-1) = ${running.positive(-1n)}`,
);

// -- and what needs adapters -----------------------------------------------
//
// A string crosses as a pointer and a length into memory the caller asked the
// callee to allocate, and comes back through a return area. This backend
// passes one address to a header and some bytes. `cabi_realloc` and the two
// halves of a string are what stand between them.
//
// Four exports rather than one, because the ways this can be wrong are not the
// same way: a string that is not the first parameter, two of them at once, a
// string going one way only, and one export in the same component that needs
// no adapter at all and must not be given one.
const TEXT = `module greeter

fn greet(name: String) -> String { join(["hello, ", name], "") }

fn tag(n: Int, name: String, on: Bool) -> String {
    join([name, "-", to_string(n)], "")
}

fn both(a: String, b: String) -> String { join([a, "|", b], "") }

fn width(text: String) -> Int { length(text) }

fn plain(n: Int) -> Int { n + 1 }
`;

const text = join(scratch, "greeter.deed");
writeFileSync(text, TEXT);
execFileSync(deed, ["build", "--component", text], { stdio: "pipe" });

const greeter = join(scratch, "greeter.component.wasm");
const carrying = jco(["wit", greeter], { encoding: "utf8" });
check(
  "the world of a component carrying text says string on both sides",
  carrying.includes("export greet: func(p0: string) -> string"),
  carrying.trim(),
);
check(
  "and says it for a string that is not the first parameter",
  carrying.includes("export tag: func(p0: s64, p1: string, p2: bool) -> string"),
  carrying.trim(),
);

jco(["transpile", greeter, "-o", join(scratch, "text"), "--name", "greeter"], { stdio: "pipe" });
const said = await import(pathToFileURL(resolve(scratch, "text", "greeter.js")).href);

check("a component runtime hands it a string", said.greet("world") === "hello, world", said.greet("world"));
check("an empty one too", said.greet("") === "hello, ", JSON.stringify(said.greet("")));
check(
  "a string among other parameters lands in the right place",
  said.tag(7n, "row", true) === "row-7",
  said.tag(7n, "row", true),
);
check("two of them do not overwrite each other", said.both("left", "right") === "left|right", said.both("left", "right"));

// The count the layout carries is characters and the boundary carries bytes,
// so a wrapper that copied the byte count would be right for every string in
// this file that is ASCII and wrong for the rest.
const wide = "dünya 日本語";
check(
  "and text outside ASCII arrives with the right character count",
  said.width(wide) === BigInt([...wide].length),
  `width(${wide}) = ${said.width(wide)}, and it has ${[...wide].length} characters`,
);
check("while text going only one way still crosses", said.greet(wide) === `hello, ${wide}`, said.greet(wide));

// Enough to need more memory than a module starts with, which is what a
// `cabi_realloc` that moved the bump pointer without growing would fail at.
// `str_concat` had exactly that bug for two releases.
const long = "x".repeat(300_000);
check(
  "and a string past the pages the module starts with",
  said.greet(long).length === long.length + 7,
  `${said.greet(long).length} characters back from ${long.length}`,
);

check("an export needing no adapter still answers in a component that has them", said.plain(41n) === 42n, said.plain(41n));

// -- and what still has none ------------------------------------------------
//
// A list crosses as a pointer and a length as well, and the elements behind it
// have to be lowered one at a time. Nothing here does that, so the component is
// not written rather than written wrongly.
const LIST = `module rows

fn firsts(rows: List<String>) -> String { join(rows, ",") }
`;

const list = join(scratch, "rows.deed");
writeFileSync(list, LIST);
const refused = execFileSync(deed, ["build", "--component", list], { encoding: "utf8" });
check(
  "a module carrying a list is told which export needs adapters that are not written",
  refused.includes("no component binary") &&
    refused.includes("firsts") &&
    refused.includes("canonical ABI"),
  JSON.stringify(refused),
);

let wrote = true;
try {
  readFileSync(join(scratch, "rows.component.wasm"));
} catch {
  wrote = false;
}
check("and is not given a component that would answer wrongly", !wrote);

// The world and the core module are still written for it, because the
// derivation is the claim and it does not depend on the adapters.
check(
  "while the world it derived is still there",
  readFileSync(join(scratch, "rows.wit"), "utf8").includes("export firsts:"),
);

console.log(
  failures === 0
    ? "\nmeasured: components that run, with and without the adapters, and a refusal where they are missing"
    : `\n${failures} failed`,
);
process.exit(failures === 0 ? 0 : 1);
