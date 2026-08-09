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
// That gap is closed for the exports that need no adapters, so this file now
// measures the thing working rather than the thing missing. Both halves are
// here, because the second one is still true:
//
//   - a component `deed build --component` wrote, whose world names the
//     functions, and which answers correctly when a component runtime calls
//     it;
//   - and a module carrying text, which is not given a component at all,
//     because the canonical ABI adapters a string needs are not written.
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

// -- and what does not ------------------------------------------------------
//
// A string crosses as a pointer and a length into memory the caller helped
// allocate, through `cabi_realloc`. This backend passes one address in its own
// layout. Lifting that anyway is a component that answers wrongly, which is
// worse than one that is not written.
const TEXT = `module greeter

fn greet(name: String) -> String { join(["hello, ", name], "") }
`;

const text = join(scratch, "greeter.deed");
writeFileSync(text, TEXT);
const said = execFileSync(deed, ["build", "--component", text], { encoding: "utf8" });
check(
  "a module carrying text is told which export needs the adapters",
  said.includes("no component binary") && said.includes("greet") && said.includes("canonical ABI"),
  JSON.stringify(said),
);

let wrote = true;
try {
  readFileSync(join(scratch, "greeter.component.wasm"));
} catch {
  wrote = false;
}
check("and is not given a component that would answer wrongly", !wrote);

// The world and the core module are still written for it, because the
// derivation is the claim and it does not depend on the adapters.
check(
  "while the world it derived is still there",
  readFileSync(join(scratch, "greeter.wit"), "utf8").includes("export greet:"),
);

console.log(
  failures === 0
    ? "\nmeasured: a component that runs, and a refusal where the adapters are missing"
    : `\n${failures} failed`,
);
process.exit(failures === 0 ? 0 : 1);
