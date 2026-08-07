// Does an engine that is not this repository's run what `deed build` wrote?
//
// `crates/deed-codegen/src/run.rs` is a test oracle. It is small, it is
// permissive, and it shares an address space with the host it dispatches to,
// so it can hand a host implementation the module's memory directly. No real
// embedder can do that. A module only this workspace's runner accepts is a
// module every other engine rejects, and #776 is what that costs: a released
// artifact that trapped on every input, because CI built it and weighed it
// and never ran it.
//
// So this runs one, in the engine Node ships, through the same door anybody
// else would use: compile the bytes, read the import section the engine sees,
// supply exactly those, call the export.
//
//   node crates/deed-codegen/smoke.mjs <path to deed> <scratch directory>

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [deed, scratch] = process.argv.slice(2);
if (!deed || !scratch) {
  console.error("usage: smoke.mjs <path to deed> <scratch directory>");
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

// A program that carries text across the boundary in both directions: a
// literal it wrote and an argument it was handed. Numbers alone would pass
// without the module exporting its memory, which is the thing that was
// missing.
const SOURCE = `module smoke

fn greet(out: Console, name: String) -> ()
  uses
    Io.write,
{
    Io.write(out, "hello, ")
    Io.write(out, name)
}

fn main(sys: System) -> Int
  uses
    Io.write,
{
    greet(sys.console, "world")
    21 + 21
}
`;

mkdirSync(scratch, { recursive: true });
const source = join(scratch, "smoke.deed");
writeFileSync(source, SOURCE);
execFileSync(deed, ["build", source], { stdio: "pipe" });
const bytes = readFileSync(join(scratch, "smoke.wasm"));

// The engine's own reading of the binary, not ours.
const module = await WebAssembly.compile(bytes);
check("a real engine compiles what `deed build` wrote", true);

const imports = WebAssembly.Module.imports(module).map(
  (one) => `${one.module}.${one.name}`,
);
const exports = WebAssembly.Module.exports(module);

// The import section is the row, which is the claim `design/05-backend.md`
// makes about every compiled program.
check(
  "the engine sees the row as the import section",
  imports.includes("deed:io.write") && imports.includes("deed:sys.console"),
  JSON.stringify(imports),
);
check(
  "and nothing the program does not use",
  !imports.some((one) => one.startsWith("deed:io.read")),
  JSON.stringify(imports),
);
check(
  "the memory is exported, so a host can read a string it is handed",
  exports.some((one) => one.name === "memory" && one.kind === "memory"),
  JSON.stringify(exports.map((one) => `${one.name}:${one.kind}`)),
);

// Everything below reads the module's memory. Without the export there is
// nothing to read, and going on would report a crash in this file instead of
// the finding above it.
if (failures > 0) {
  console.log("\nnothing outside this module can read what it hands over");
  process.exit(1);
}

// A host, written the way an embedder outside this repository would have to
// write one: a table of things it kept, and handles that index it.
const held = ["System", "Console"];
const handle = (what) => BigInt(held.indexOf(what) + 1);
const stands = (handed) => held[Number(handed) - 1];

let memory = null;
const written = [];
let refused = 0;

function text(address) {
  const view = new DataView(memory.buffer);
  const at = Number(address);
  const bytes = Number(view.getBigUint64(at + 8, true));
  return new TextDecoder().decode(new Uint8Array(memory.buffer, at + 16, bytes));
}

const instance = await WebAssembly.instantiate(module, {
  "deed:sys": {
    console: (sys) => {
      if (stands(sys) !== "System") {
        refused += 1;
        throw new Error("not the root");
      }
      return handle("Console");
    },
  },
  "deed:io": {
    write: (console_, address) => {
      if (stands(console_) !== "Console") {
        refused += 1;
        throw new Error("not the console");
      }
      written.push(text(address));
    },
  },
});
check("and instantiates it against a host offering exactly that", true);

memory = instance.exports.memory;
const answer = instance.exports.main(handle("System"));

check("`main` runs to the end", answer === 42n, String(answer));
check(
  "and the host was handed the text, not an address it could not read",
  written.join("") === "hello, world",
  JSON.stringify(written),
);

// The other half. A capability is a number, and the module's memory is full
// of numbers, so a host has to look its arguments up rather than trust them.
try {
  instance.exports.main(9999n);
  check("a handle the host never gave out is refused", false, "it was accepted");
} catch {
  check("a handle the host never gave out is refused", refused > 0);
}

console.log(
  failures === 0
    ? "\nan engine that is not this one ran it"
    : `\n${failures} failed`,
);
process.exit(failures === 0 ? 0 : 1);
