// What a real component toolchain makes of what `deed build --component` wrote.
//
// The claim `deed build --component` is for is `design/05-backend.md`'s: a
// program's effect row is its world, and nothing else derives one. The world
// is written as `.wit` text, and nothing in this repository reads it back,
// which is the shape of a claim that has never been tested by anybody who
// would have to believe it.
//
// So this asks the toolchain that would. `jco` bundles the Bytecode
// Alliance's own `wasm-tools`, and `jco new` is `wasm-tools component new`:
// the step somebody adopting the component model would run next.
//
// It is a measurement rather than a gate. What it pins is what is true today,
// so that the day it stops being true is the day this file changes and
// somebody has to say why.
//
//   node crates/deed-codegen/component.mjs <path to deed> <scratch directory>

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

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

// No `main` and no capability in a signature: what a component is allowed to
// be. One function of numbers and one of text, because the two cross the
// boundary differently and only one of them is a word.
const SOURCE = `module adder

fn add(a: Int, b: Int) -> Int { a + b }

fn greet(name: String) -> String { "hello, " + name }
`;

mkdirSync(scratch, { recursive: true });
const source = join(scratch, "adder.deed");
writeFileSync(source, SOURCE);
execFileSync(deed, ["build", "--component", source], { stdio: "pipe" });

const world = readFileSync(join(scratch, "adder.wit"), "utf8");
check(
  "the world names both functions the module declares",
  world.includes("export add:") && world.includes("export greet:"),
  JSON.stringify(world),
);

// The core module is a core module, which is the half that already works.
const core = readFileSync(join(scratch, "adder.wasm"));
const module = await WebAssembly.compile(core);
const exports = WebAssembly.Module.exports(module).map((one) => one.name);
check(
  "and the core module exports them",
  exports.includes("add") && exports.includes("greet"),
  JSON.stringify(exports),
);

// The step somebody adopting the component model runs next. Through a shell,
// because npm installs it as a wrapper script and Node will not spawn one
// directly on Windows.
const jco = (args, options = {}) =>
  execFileSync("jco", args, { shell: true, ...options });

const componentised = join(scratch, "adder.component.wasm");
jco(["new", join(scratch, "adder.wasm"), "-o", componentised], { stdio: "pipe" });
check("a component toolchain accepts the core module", true);

const produced = jco(["wit", componentised], { encoding: "utf8" });

// What is true today, and the reason `--component` does not claim to write a
// component binary. The exports cross the boundary in this backend's own
// layout rather than the canonical ABI, and nothing writes the
// component-type custom section that carries the world, so the component the
// toolchain builds has nothing in it.
//
// The day this stops being true, this file is the one that fails, and the
// assertion below is the one to rewrite.
const empty = !produced.includes("add") && !produced.includes("greet");
check(
  "and the component it builds has an empty world, which is the gap",
  empty,
  produced.trim(),
);

if (!empty) {
  console.log(
    "\nthe world is not empty any more. If that was on purpose, this file and\n" +
      "design/decisions/2026-08-07-a-wit-world-is-not-a-component.md are what to update.",
  );
}

console.log(
  failures === 0
    ? "\nmeasured: a world in text, and a component with nothing in it"
    : `\n${failures} failed`,
);
process.exit(failures === 0 ? 0 : 1);
