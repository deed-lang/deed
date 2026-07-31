// Loads the built wasm artifact and calls it, which nothing else does.
//
// The crate's own tests run on the host, where `Instant::now` works and this
// module's target is never exercised. That gap shipped a release whose
// artifact trapped on every input (#776). Building it and weighing it is not
// the same as running it, so CI runs this.
//
// Node rather than a wasm runtime crate, because this repository has no
// dependencies and a runner already has node.

import { readFile } from "node:fs/promises";

const path = process.argv[2];
if (!path) {
  console.error("usage: node smoke.mjs <path to deed_wasm.wasm>");
  process.exit(2);
}

const { instance } = await WebAssembly.instantiate(await readFile(path), {});
const wasm = instance.exports;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

// `memory.buffer` detaches whenever the module grows its heap, so every read
// takes a fresh view rather than holding one.
const bytes = () => new Uint8Array(wasm.memory.buffer);

function readResult() {
  const ptr = wasm.deed_result_ptr();
  const len = wasm.deed_result_len();
  const text = decoder.decode(bytes().slice(ptr, ptr + len));
  wasm.deed_free(ptr, len);
  return text;
}

function call(verb, source) {
  const input = encoder.encode(source);
  const ptr = wasm.deed_alloc(input.length);
  bytes().set(input, ptr);
  wasm[verb](ptr, input.length);
  const result = readResult();
  wasm.deed_free(ptr, input.length);
  return result;
}

const failures = [];
const check = (what, condition, saw) => {
  if (condition) return;
  failures.push(`${what}\n    saw: ${JSON.stringify(saw)}`);
};

const HELLO = `module main

fn main(sys: System) -> Int
  uses
    Io.write,
{
    Io.write(sys.console, "hi")
    1
}
`;

wasm.deed_version();
const version = readResult();
check("deed_version reports something", /^\d+\.\d+\.\d+$/.test(version), version);

// One line per JSON object, and every line parses. A program printing a quote
// is the case that used to produce something no caller could read (#768).
const printsAQuote = `module main

fn main(sys: System) -> Int
  uses
    Io.write,
{
    Io.write(sys.console, "he said \\"hi\\"")
    0
}
`;
for (const [verb, source] of [
  ["deed_check", HELLO],
  ["deed_run", HELLO],
  ["deed_test", 'module main\n\ntest "one" {\n    assert 1 == 1\n}\n'],
  ["deed_fmt", HELLO],
  ["deed_run", printsAQuote],
]) {
  const answer = call(verb, source);
  for (const line of answer.split("\n").filter((l) => l.trim() !== "")) {
    try {
      JSON.parse(line);
    } catch (error) {
      failures.push(`${verb} wrote a line no caller can parse: ${error}\n    saw: ${line}`);
    }
  }
}

// The four verbs answer different questions, so each one is checked for the
// answer only it gives rather than for "did not trap".
check("check is quiet about a clean program", call("deed_check", HELLO).includes('"kind":"obligation"') || call("deed_check", HELLO) === "", "");
check("run reports what the program printed", call("deed_run", HELLO).includes('"kind":"output"'), call("deed_run", HELLO));
check("test reports a passing test", call("deed_test", 'module main\n\ntest "one" {\n    assert 1 == 1\n}\n').includes('"passed":true'), "");
check("fmt hands back a program", call("deed_fmt", "module main\n\n\n\nfn main( ) -> Int {\n  1\n}\n").includes('"kind":"formatted"'), "");
check(
  "a program that does not parse is refused rather than reshaped",
  !call("deed_fmt", "module main\n\nfn main( -> Int {\n").includes('"kind":"formatted"'),
  "",
);
check(
  "a diagnostic reaches the caller",
  call("deed_check", "module main\n\nfn main() -> Int {\n    nonesuch\n}\n").includes('"kind":"diagnostic"'),
  "",
);

if (failures.length > 0) {
  console.error(`the artifact does not answer:\n\n${failures.join("\n\n")}`);
  process.exit(1);
}

console.log(`the artifact answers: deed ${version}, four verbs, every line parses`);
