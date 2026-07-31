//! The `deed` command line tool.

mod args;

use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use deed_ast::Item;
use deed_diagnostics::{Diagnostic, FileId, SourceMap, render_human};
use deed_driver::{Checked, ObligationReport};
use deed_interp::{Program, PropertyConfig, RuntimeProfile};
use deed_typeck::Tier;

use crate::args::{CheckArgs, Command, Format, Mode, USAGE};

/// Something went wrong with the invocation rather than with the code.
const EXIT_USAGE: u8 = 2;

/// How much stack the work gets.
///
/// Every pass walks the syntax tree by recursion, and so does the interpreter,
/// so the depth a Deed program can reach is bounded by the host stack rather
/// than by anything in the language. The interpreter has its own limit and
/// reports hitting it, but that limit is only meaningful if there is room to
/// reach it. The main thread's stack is whatever the platform felt like.
const STACK: usize = 64 * 1024 * 1024;

fn main() -> ExitCode {
    // Everything runs here, and nothing crosses back, so none of the values
    // the compiler builds have to be shared between threads.
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(run)
        .and_then(|worker| worker.join().map_err(|_| io::Error::other("worker died")))
        .unwrap_or_else(|error| {
            eprintln!("error: {error}");
            ExitCode::from(EXIT_USAGE)
        })
}

fn run() -> ExitCode {
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    match command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("deed {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Explain(code) => run_explain(&code),
        Command::Check(check) => run_check(check),
        Command::Lsp => run_lsp(),
    }
}

/// Prints the generated page for one diagnostic code.
///
/// The argument may be the code identifier (`DEED4025`) or the constant name
/// (`BROKEN_PRECONDITION`).  The content comes from the build-generated data
/// in `deed-explain`, which in turn comes from the doc-comment lines already
/// above each `pub const` in a `codes.rs` file and from an example extracted
/// from the test corpus.
fn run_explain(query: &str) -> ExitCode {
    let Some(p) = deed_explain::page(query) else {
        eprintln!("error: no page for `{query}`");
        eprintln!("       use the code identifier (e.g. DEED4025) or the constant name");
        return ExitCode::from(EXIT_USAGE);
    };

    println!("{} {}", p.code, p.name);
    println!();

    if !p.text.is_empty() {
        println!("{}", p.text);
    }

    if let (Some(example), Some(source)) = (p.example, p.example_source) {
        println!();
        println!("Example (from {source}):");
        println!();
        for line in example.lines() {
            println!("    {line}");
        }
    }

    ExitCode::SUCCESS
}

/// Speaks the language server protocol until the editor stops.
///
/// Nothing is printed to stdout that is not a protocol message, because stdout
/// is the protocol. A stray `println!` anywhere in this path would desync the
/// editor and the failure would look like the server hanging.
fn run_lsp() -> ExitCode {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();

    match deed_lsp::serve(&mut input, &mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn run_check(args: CheckArgs) -> ExitCode {
    let mut files = Vec::new();
    for path in &args.paths {
        if let Err(error) = collect(path, &mut files) {
            eprintln!("error: {}: {error}", path.display());
            return ExitCode::from(EXIT_USAGE);
        }
    }

    if files.is_empty() {
        eprintln!("error: no `.deed` files found");
        return ExitCode::from(EXIT_USAGE);
    }

    // Deterministic order, so output can be diffed between runs.
    files.sort();
    files.dedup();

    if args.mode == Mode::Fmt {
        return run_fmt(&files, args.check_only);
    }
    if args.mode == Mode::Fix {
        return run_fix(&files, args.check_only);
    }
    if args.runtime_profile && args.mode != Mode::Run {
        eprintln!("error: `--profile-runtime` is only for `deed run`");
        return ExitCode::from(EXIT_USAGE);
    }

    // What was named is the subject; what an import needed is context. So the
    // library a program uses is compiled alongside it and checked, and its
    // tests and its `main` are not the ones you asked about.
    let subject = files.len();
    let mut shipped: Vec<&'static str> = Vec::new();
    let mut manifests: Vec<(String, String, Vec<Diagnostic>)> = Vec::new();
    if let Err(error) = resolve_imports(&mut files, &mut shipped, &mut manifests) {
        eprintln!("error: {error}");
        return ExitCode::from(EXIT_USAGE);
    }

    let mut sources = SourceMap::new();
    let mut ids = Vec::new();

    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: {}: {error}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };
        ids.push(sources.add(display_path(path), text));
    }

    // Last, so that the subject is still the first `subject` of them. A module
    // that came out of the compiler is context by definition: nobody named it.
    for module in &shipped {
        let Some(text) = deed_driver::shipped_source(module) else {
            continue;
        };
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    // Register manifest files in the source map so their text appears in
    // diagnostics, then re-anchor each diagnostic to the registered file id.
    let manifest_diagnostics: Vec<Vec<Diagnostic>> = manifests
        .into_iter()
        .map(|(name, text, diagnostics)| {
            let file = sources.add(name, text);
            diagnostics
                .into_iter()
                .map(|mut d| {
                    d.file = file;
                    d
                })
                .collect()
        })
        .collect();

    // Every file at once, so a `use` has something to point at. Checking them
    // one at a time would mean an import could never resolve, which is how it
    // used to work and why nothing crossing a module boundary was checked.
    let checks = deed_driver::check_all(&sources, &ids);

    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Manifest diagnostics first, in source order within each manifest file.
    let manifest_has_errors = manifest_diagnostics
        .iter()
        .flatten()
        .any(Diagnostic::is_error);
    for diagnostics in &manifest_diagnostics {
        for diagnostic in diagnostics {
            if let Err(error) = writeln!(out, "{}", render_human(&sources, diagnostic)) {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    let result = match args.format {
        Format::Human => report_human(&mut out, &sources, &checks, args.obligations, args.timings),
        Format::Json => report_json(&mut out, &sources, &checks, args.obligations),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        return ExitCode::from(EXIT_USAGE);
    }

    let errors: usize = checks.iter().map(Checked::error_count).sum();
    if errors > 0 || manifest_has_errors {
        return ExitCode::FAILURE;
    }

    if args.mode == Mode::Test {
        // Running code that does not check would be answering a question
        // nobody asked, and the failure would be about the wrong thing.
        let test_result = if args.compiled {
            run_compiled_tests(&mut out, &sources, &checks, subject)
        } else {
            run_tests(&mut out, &sources, &checks, subject)
        };
        match test_result {
            Ok(true) => {}
            Ok(false) => return ExitCode::FAILURE,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    if args.mode == Mode::Run {
        match if args.compiled {
            run_compiled_main(&mut out, &sources, &checks, subject, &args.arguments)
        } else {
            run_main(
                &mut out,
                &sources,
                &checks,
                subject,
                args.dir.as_deref(),
                &args.arguments,
                args.runtime_profile,
            )
        } {
            Ok(Some(true)) => {}
            Ok(Some(false)) => return ExitCode::FAILURE,
            Ok(None) => return ExitCode::from(EXIT_USAGE),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    if args.mode == Mode::Build {
        match build(&mut out, &files, &checks, subject, args.component) {
            Ok(true) => {}
            Ok(false) => return ExitCode::FAILURE,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    ExitCode::SUCCESS
}

/// Compiles each named file to a WebAssembly module beside it, or to a
/// component (module plus WIT world) when `component` is true.
///
/// Only the files somebody named. A module that came in because an import
/// wanted it is context, and compiling it would write a file nobody asked
/// for next to somebody else's source.
///
/// What it cannot compile it says, by function and by what it found, and
/// answers with a failure. The interpreter still runs all of it, which is
/// what `design/05-backend.md` means by the interpreter staying.
fn build(
    out: &mut impl Write,
    files: &[PathBuf],
    checks: &[Checked],
    subject: usize,
    component: bool,
) -> io::Result<bool> {
    if component {
        return build_component(out, files, checks, subject);
    }

    let mut wrote = false;

    for (path, checked) in files.iter().zip(checks).take(subject) {
        let lowered = match deed_mir::lower(&checked.module, &checked.resolutions, &checked.types) {
            Ok(lowered) => lowered,
            Err(why) => {
                writeln!(out, "{}: {why}", path.display())?;
                continue;
            }
        };
        let module = match deed_codegen::compile(&lowered) {
            Ok(module) => module,
            Err(why) => {
                writeln!(out, "{}: {why}", path.display())?;
                continue;
            }
        };

        let target = path.with_extension("wasm");
        std::fs::write(&target, module.encode())?;
        writeln!(out, "{}", target.display())?;
        wrote = true;
    }

    if !wrote {
        writeln!(out, "nothing was compiled")?;
        return Ok(false);
    }
    Ok(true)
}

/// Compiles each named file to a WebAssembly module and a WIT world file.
///
/// A component has an interface rather than a `main`. Every function the
/// module declares is part of that interface; tests are not. A module that
/// declares `main` is a program, not a component, and is refused here with
/// a message pointing at `deed build` instead.
///
/// Functions whose signatures include a capability have no world-level type
/// in WIT and are refused too, with a per-function explanation. Anything the
/// regular backend cannot compile is refused on the same terms as `deed build`.
///
/// On success, two files are written beside the source: `<name>.wasm` holds
/// the module, `<name>.wit` holds the WIT world that describes its interface.
fn build_component(
    out: &mut impl Write,
    files: &[PathBuf],
    checks: &[Checked],
    subject: usize,
) -> io::Result<bool> {
    let mut wrote = false;

    for (path, checked) in files.iter().zip(checks).take(subject) {
        // Step 1: Lower to MIR. The same pass `deed build` does; refusing here
        // rather than after avoids writing a WIT for a module that cannot compile.
        let lowered = match deed_mir::lower(&checked.module, &checked.resolutions, &checked.types) {
            Ok(lowered) => lowered,
            Err(why) => {
                writeln!(out, "{}: {why}", path.display())?;
                continue;
            }
        };

        // Step 2: A module with `main` is a program. Components have an
        // interface instead.
        let has_main = checked.module.items.iter().any(|item| {
            if let Item::Function(f) = item {
                f.sig.name.name.as_str() == "main"
            } else {
                false
            }
        });
        if has_main {
            writeln!(
                out,
                "{}: declares `main`, which is a program entry point; \
                 a component has an interface instead of a `main`; \
                 use `deed build` for programs",
                path.display()
            )?;
            continue;
        }

        // Step 3: Check every function signature for types that have no
        // world-level counterpart in WIT.
        let mut refused = false;
        for function in &lowered.functions {
            for (i, param_ty) in function.params.iter().enumerate() {
                if let Some(why) = wit_incompatible(param_ty) {
                    writeln!(
                        out,
                        "{}: `{}` parameter {} is {why}, which has no world-level type in WIT",
                        path.display(),
                        function.name,
                        i,
                    )?;
                    refused = true;
                }
            }
            if let Some(why) = wit_incompatible(&function.ret) {
                writeln!(
                    out,
                    "{}: `{}` returns {why}, which has no world-level type in WIT",
                    path.display(),
                    function.name,
                )?;
                refused = true;
            }
        }
        if refused {
            continue;
        }

        // Step 4: Compile to WebAssembly; the same step as `deed build`.
        let module = match deed_codegen::compile(&lowered) {
            Ok(module) => module,
            Err(why) => {
                writeln!(out, "{}: {why}", path.display())?;
                continue;
            }
        };

        // Step 5: Generate the WIT world.
        let module_name = component_name(&checked.module);
        let wit = generate_wit(&module_name, &lowered);

        // Step 6: Write both files.
        let wasm_target = path.with_extension("wasm");
        let wit_target = path.with_extension("wit");
        std::fs::write(&wasm_target, module.encode())?;
        std::fs::write(&wit_target, wit.as_bytes())?;
        writeln!(out, "{}", wasm_target.display())?;
        writeln!(out, "{}", wit_target.display())?;
        wrote = true;
    }

    if !wrote {
        writeln!(out, "nothing was compiled")?;
        return Ok(false);
    }
    Ok(true)
}

/// Returns a human-readable description of why this type has no WIT
/// world-level counterpart, or `None` if it maps cleanly.
fn wit_incompatible(ty: &deed_mir::Ty) -> Option<&'static str> {
    match ty {
        deed_mir::Ty::Capability => Some("a capability"),
        deed_mir::Ty::Closure => Some("a function value"),
        deed_mir::Ty::List(elem) => wit_incompatible(elem),
        _ => None,
    }
}

/// Maps a MIR type to its WIT spelling.
///
/// Caller must have already checked [`wit_incompatible`] and knows the type
/// is representable; panics on `Capability` and `Closure`.
fn to_wit_type(ty: &deed_mir::Ty, program: &deed_mir::Program) -> String {
    match ty {
        deed_mir::Ty::Unit => "unit".to_string(),
        deed_mir::Ty::Bool => "bool".to_string(),
        deed_mir::Ty::Int => "s64".to_string(),
        deed_mir::Ty::Str => "string".to_string(),
        deed_mir::Ty::List(elem) => format!("list<{}>", to_wit_type(elem, program)),
        deed_mir::Ty::Aggregate(id) => to_kebab(&program.layout(*id).name),
        deed_mir::Ty::Capability | deed_mir::Ty::Closure => {
            unreachable!("incompatible types are caught before this point")
        }
    }
}

/// Converts snake_case to kebab-case, as WIT identifiers require.
fn to_kebab(s: &str) -> String {
    s.replace('_', "-")
}

/// The module declaration name, or a fallback derived from the first segment.
///
/// WIT package names use `namespace:name`, so `deed` is the namespace and the
/// module path becomes the name. Slashes separate segments in a module path
/// (`my/module`) and are replaced with hyphens in the WIT name (`deed:my-module`),
/// since WIT uses hyphens as the segment separator within a package name.
fn component_name(module: &deed_ast::Module) -> String {
    match &module.name {
        Some(path) => path.to_string_path().replace(['/', '_'], "-"),
        None => "unnamed".to_string(),
    }
}

/// Generates a WIT world file describing the module's exported interface.
///
/// Every function the module declares (excluding `main`, which was already
/// refused) becomes an export in the `component` world. Record and choice
/// types that appear in signatures are declared before the world.
fn generate_wit(module_name: &str, program: &deed_mir::Program) -> String {
    let mut out = String::new();

    // Package declaration.
    let _ = writeln!(out, "package deed:{module_name};");
    let _ = writeln!(out);

    // Collect layouts that appear in any function signature, in the order
    // they are first encountered, so the output is deterministic.
    let mut needed: Vec<deed_mir::LayoutId> = Vec::new();
    for function in &program.functions {
        for ty in function.params.iter().chain(std::iter::once(&function.ret)) {
            collect_layouts(ty, program, &mut needed);
        }
    }

    // Emit type definitions before the world block that uses them.
    for id in &needed {
        let layout = program.layout(*id);
        let is_record = layout.variants.len() == 1 && layout.variants[0].name == layout.name;
        if is_record {
            let variant = &layout.variants[0];
            let _ = writeln!(out, "record {} {{", to_kebab(&layout.name));
            for field in &variant.fields {
                let _ = writeln!(
                    out,
                    "    {}: {},",
                    to_kebab(&field.name),
                    to_wit_type(&field.ty, program)
                );
            }
            let _ = writeln!(out, "}}");
        } else {
            // Choice type: emit auxiliary records for variants with multiple
            // fields, then the variant itself.
            for variant in &layout.variants {
                if variant.fields.len() > 1 {
                    let aux = format!("{}-{}", to_kebab(&layout.name), to_kebab(&variant.name));
                    let _ = writeln!(out, "record {aux} {{");
                    for field in &variant.fields {
                        let _ = writeln!(
                            out,
                            "    {}: {},",
                            to_kebab(&field.name),
                            to_wit_type(&field.ty, program)
                        );
                    }
                    let _ = writeln!(out, "}}");
                }
            }
            let _ = writeln!(out, "variant {} {{", to_kebab(&layout.name));
            for variant in &layout.variants {
                match variant.fields.len() {
                    0 => {
                        let _ = writeln!(out, "    {},", to_kebab(&variant.name));
                    }
                    1 => {
                        let _ = writeln!(
                            out,
                            "    {}({}),",
                            to_kebab(&variant.name),
                            to_wit_type(&variant.fields[0].ty, program)
                        );
                    }
                    _ => {
                        let aux = format!("{}-{}", to_kebab(&layout.name), to_kebab(&variant.name));
                        let _ = writeln!(out, "    {}({aux}),", to_kebab(&variant.name));
                    }
                }
            }
            let _ = writeln!(out, "}}");
        }
        let _ = writeln!(out);
    }

    // World block with one export per function.
    let _ = writeln!(out, "world component {{");
    for function in &program.functions {
        // Synthetic parameter names: WIT requires names, but the MIR does not
        // preserve them. `p0`, `p1`, ... are unambiguous and stable.
        let params: Vec<String> = function
            .params
            .iter()
            .enumerate()
            .filter(|(_, ty)| !matches!(ty, deed_mir::Ty::Unit))
            .map(|(i, ty)| format!("p{i}: {}", to_wit_type(ty, program)))
            .collect();
        let params_str = params.join(", ");

        match &function.ret {
            deed_mir::Ty::Unit => {
                let _ = writeln!(
                    out,
                    "    export {}: func({params_str});",
                    to_kebab(&function.name)
                );
            }
            ret => {
                let ret_str = to_wit_type(ret, program);
                let _ = writeln!(
                    out,
                    "    export {}: func({params_str}) -> {ret_str};",
                    to_kebab(&function.name)
                );
            }
        }
    }
    let _ = writeln!(out, "}}");

    out
}

/// Collects [`deed_mir::LayoutId`]s that appear in a type, in encounter order,
/// without duplicates.
fn collect_layouts(
    ty: &deed_mir::Ty,
    program: &deed_mir::Program,
    out: &mut Vec<deed_mir::LayoutId>,
) {
    match ty {
        deed_mir::Ty::Aggregate(id) => {
            if !out.contains(id) {
                // Recurse into field types before adding this layout, so
                // dependencies are declared before the type that uses them.
                let layout = program.layout(*id);
                for variant in &layout.variants {
                    for field in &variant.fields {
                        collect_layouts(&field.ty, program, out);
                    }
                }
                out.push(*id);
            }
        }
        deed_mir::Ty::List(elem) => collect_layouts(elem, program, out),
        _ => {}
    }
}

///
/// Files are rewritten in place only when the content actually changes, so a
/// no-op run does not touch mtimes and trigger every watcher on the machine.
fn run_fmt(files: &[PathBuf], check_only: bool) -> ExitCode {
    let mut sources = SourceMap::new();
    let mut unformatted = Vec::new();
    let mut failed = false;

    for path in files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: {}: {error}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };

        let file = sources.add(display_path(path), text.clone());
        let formatted = match deed_fmt::format(file, &text) {
            Ok(formatted) => formatted,
            Err(diagnostics) => {
                // Reshaping a file that does not parse would be guessing at
                // what was meant, and the guess lands in the working tree.
                for diagnostic in &diagnostics {
                    println!("{}", render_human(&sources, diagnostic));
                }
                failed = true;
                continue;
            }
        };

        if formatted == text {
            continue;
        }
        if check_only {
            unformatted.push(path.clone());
            continue;
        }
        if let Err(error) = std::fs::write(path, &formatted) {
            eprintln!("error: {}: {error}", path.display());
            return ExitCode::from(EXIT_USAGE);
        }
        println!("{}", display_path(path));
    }

    if failed {
        return ExitCode::FAILURE;
    }
    if !unformatted.is_empty() {
        for path in &unformatted {
            println!("{}", display_path(path));
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Every checked module, so the interpreter can follow a call out of one.
fn program_of(checks: &[Checked]) -> Program<'_> {
    let mut program = Program::new();
    for checked in checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }
    program
}

fn compiled_diagnostic(file: FileId, trap: &deed_codegen::Trap) -> Option<Diagnostic> {
    let deed_codegen::Trap::Failed {
        code,
        message,
        span: Some(span),
        ..
    } = trap
    else {
        return None;
    };

    let (code, label) = match code.as_str() {
        deed_mir::codes::ASSERTION_FAILED => {
            (deed_mir::codes::ASSERTION_FAILED, "evaluated to false")
        }
        deed_mir::codes::PRECONDITION_FAILED => {
            (deed_mir::codes::PRECONDITION_FAILED, "precondition not met")
        }
        deed_mir::codes::NOT_RUNNABLE => (deed_mir::codes::NOT_RUNNABLE, "not runnable"),
        _ => return None,
    };

    Some(Diagnostic::error(code, file, *span, message.clone()).with_primary_label(label))
}

fn compiled_args(module: &deed_codegen::Module, name: &str) -> Option<Vec<deed_codegen::Value>> {
    let index = module
        .exports
        .iter()
        .find(|(exported, _)| exported == name)
        .map(|(_, index)| *index)?;
    let type_index = match (index as usize).checked_sub(module.imports.len()) {
        None => module.imports[index as usize].type_index,
        Some(at) => module.funcs.get(at)?.type_index,
    };
    let ty = module.types.get(type_index as usize)?;
    Some(
        ty.params
            .iter()
            .map(|param| match param {
                deed_codegen::wasm::ValType::I32 => deed_codegen::Value::I32(0),
                deed_codegen::wasm::ValType::I64 => deed_codegen::Value::I64(0),
            })
            .collect(),
    )
}

fn run_compiled_main(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    subject: usize,
    arguments: &[String],
) -> io::Result<Option<bool>> {
    if !arguments.is_empty() {
        eprintln!(
            "error: `deed run --compiled` does not hand arguments to the backend test runner"
        );
        return Ok(None);
    }

    let mut runs = Vec::new();
    for checked in &checks[..subject.min(checks.len())] {
        let has_main = checked.module.items.iter().any(|item| {
            matches!(
                item,
                Item::Function(function) if function.sig.name.name == "main"
            )
        });
        if !has_main {
            continue;
        }

        let lowered = match deed_mir::lower(&checked.module, &checked.resolutions, &checked.types) {
            Ok(lowered) => lowered,
            Err(why) => {
                writeln!(out, "{}: {why}", sources.file(checked.file).name())?;
                return Ok(Some(false));
            }
        };
        let module = match deed_codegen::compile(&lowered) {
            Ok(module) => module,
            Err(why) => {
                writeln!(out, "{}: {why}", sources.file(checked.file).name())?;
                return Ok(Some(false));
            }
        };
        let args = compiled_args(&module, "main").ok_or_else(|| {
            io::Error::other("compiled `main` should be exported with a known signature")
        })?;
        runs.push((
            checked.file,
            sources.file(checked.file).name().to_string(),
            module,
            args,
        ));
    }

    if runs.is_empty() {
        eprintln!("error: no `main` found, so there is nothing to run");
        return Ok(None);
    }
    if runs.len() > 1 {
        let names: Vec<&str> = runs.iter().map(|(_, name, _, _)| name.as_str()).collect();
        eprintln!("error: more than one `main`, in {}", names.join(" and "));
        return Ok(None);
    }

    let (file, _, module, args) = runs.remove(0);
    match deed_codegen::call(&module, "main", &args) {
        Ok(_) => Ok(Some(true)),
        Err(trap) => {
            if let Some(diagnostic) = compiled_diagnostic(file, &trap) {
                writeln!(out, "{}", render_human(sources, &diagnostic))?;
            } else {
                writeln!(out, "{trap}")?;
            }
            Ok(Some(false))
        }
    }
}

/// Applies the fixes that are certain and leaves the guesses alone.
///
/// One file at a time, because a fix is a rewrite of the file the diagnostic
/// points at and re-checking the whole set between rounds would multiply the
/// work by the number of files for no gain. The cost is that a fix which only
/// becomes available once another file is fixed needs a second run, which is
/// worth saying rather than hiding.
fn run_fix(files: &[PathBuf], check_only: bool) -> ExitCode {
    let mut changed = Vec::new();
    let mut gave_up = Vec::new();

    for path in files {
        let original = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: {}: {error}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };

        let name = display_path(path);
        let before = diagnose(&name, &original);
        let fixed = deed_driver::fix::fix(&original, |text| diagnose(&name, text));

        if !fixed.changed() {
            continue;
        }

        // A fix that leaves more errors than it found is a wrong fix, not a
        // partial one, so the file is left alone and the run says so.
        let after = diagnose(&name, &fixed.source);
        if deed_driver::fix::error_count(&after) > deed_driver::fix::error_count(&before) {
            eprintln!(
                "error: {name}: fixing made it worse, so nothing was written. \
                 This is a compiler bug, please report it."
            );
            return ExitCode::FAILURE;
        }

        if fixed.gave_up {
            gave_up.push(name.clone());
        }
        changed.push((name, fixed));
    }

    if changed.is_empty() {
        return ExitCode::SUCCESS;
    }

    for (name, fixed) in &changed {
        let plural = if fixed.applied == 1 { "" } else { "es" };
        println!("{name}: {} fix{plural}", fixed.applied);
    }
    for name in &gave_up {
        println!("{name}: still changing after several rounds, run `deed fix` again");
    }

    if check_only {
        return ExitCode::FAILURE;
    }

    for (name, fixed) in &changed {
        let path = files
            .iter()
            .find(|path| display_path(path) == *name)
            .expect("the name came from this list");
        if let Err(error) = std::fs::write(path, &fixed.source) {
            eprintln!("error: {}: {error}", path.display());
            return ExitCode::from(EXIT_USAGE);
        }
    }

    ExitCode::SUCCESS
}

/// Checks one file on its own, which is what a fix round needs.
fn diagnose(name: &str, text: &str) -> Vec<deed_diagnostics::Diagnostic> {
    let mut sources = SourceMap::new();
    let file = sources.add(name.to_string(), text.to_string());
    deed_driver::check(&sources, file).diagnostics
}

/// Calls `main`, handing it the one `System` there is.
///
/// Exactly one is required. Two entry points in one invocation is not a
/// program with a choice, it is a question about which one was meant, and
/// guessing is worse than asking.
fn run_main(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    subject: usize,
    dir: Option<&Path>,
    arguments: &[String],
    runtime_profile: bool,
) -> io::Result<Option<bool>> {
    let root = match dir {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir()?,
    };

    // Every checked module, so a call that goes through an import has a body
    // to walk into. The interpreter used to be handed one module and stopped
    // at the first call that left it.
    let program = program_of(checks);

    let mut runs = Vec::new();
    // Only the files that were named. A library pulled in because an import
    // needed it is not an answer to "which program did you mean".
    for checked in &checks[..subject.min(checks.len())] {
        let run = if runtime_profile {
            deed_interp::run_main_profiled(&program, checked.file, &root, arguments)
        } else {
            deed_interp::run_main(&program, checked.file, &root, arguments)
        };
        if let Some(run) = run {
            runs.push((sources.file(checked.file).name().to_string(), run));
        }
    }

    if runs.is_empty() {
        eprintln!("error: no `main` found, so there is nothing to run");
        return Ok(None);
    }
    if runs.len() > 1 {
        let names: Vec<&str> = runs.iter().map(|(name, _)| name.as_str()).collect();
        eprintln!("error: more than one `main`, in {}", names.join(" and "));
        return Ok(None);
    }

    let (_, run) = runs.remove(0);
    for line in &run.output {
        writeln!(out, "{line}")?;
    }
    if runtime_profile && let Some(profile) = &run.profile {
        report_runtime_profile(out, profile)?;
    }
    match run.result {
        Ok(_) => Ok(Some(true)),
        Err(failure) => {
            writeln!(out, "{}", render_human(sources, &failure))?;
            Ok(Some(false))
        }
    }
}

fn report_runtime_profile(out: &mut impl Write, profile: &RuntimeProfile) -> io::Result<()> {
    writeln!(
        out,
        "runtime profile: {:.2}ms total",
        profile.total.as_secs_f64() * 1000.0
    )?;
    writeln!(
        out,
        "  {:<28} {:>5} {:>10} {:>10} {:>10}",
        "function", "calls", "total", "contract", "handler"
    )?;

    let mut functions: Vec<_> = profile.functions.iter().collect();
    functions.sort_by_key(|function| std::cmp::Reverse(function.total));

    for function in functions {
        let name = format!("{}/{}", function.module, function.function);
        writeln!(
            out,
            "  {:<28} {:>5} {:>8.2}ms {:>8.2}ms {:>8.2}ms",
            name,
            function.calls,
            function.total.as_secs_f64() * 1000.0,
            function.contract.as_secs_f64() * 1000.0,
            function.handler.as_secs_f64() * 1000.0
        )?;
    }

    Ok(())
}

/// Runs every test in every file that was named. Returns whether they all
/// passed.
///
/// Named, rather than every file compiled: a library pulled in because an
/// import needed it has its own tests and they are not the ones you asked to
/// run.
fn run_tests(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    subject: usize,
) -> io::Result<bool> {
    let mut passed = 0usize;
    let mut failed = Vec::new();
    let program = program_of(checks);

    for checked in &checks[..subject.min(checks.len())] {
        let outcomes = deed_interp::run_tests(&program, checked.file);
        let properties = deed_interp::run_properties(
            &program,
            checked.file,
            &checked.module,
            &checked.resolutions,
            PropertyConfig::default(),
        );
        if outcomes.is_empty() && properties.is_empty() {
            continue;
        }

        writeln!(out, "{}", sources.file(checked.file).name())?;
        for outcome in outcomes {
            match outcome.failure {
                None => {
                    passed += 1;
                    writeln!(out, "  ok    {}", outcome.name)?;
                }
                Some(diagnostic) => {
                    writeln!(out, "  FAIL  {}", outcome.name)?;
                    failed.push((outcome.name, diagnostic));
                }
            }
        }

        // Generated from the contract rather than written by hand, so they are
        // labelled that way. Nobody should have to wonder where a failing test
        // they never wrote came from.
        for property in properties {
            let label = format!("property {} ({} cases)", property.function, property.cases);
            match property.failure {
                None => {
                    passed += 1;
                    writeln!(out, "  ok    {label}")?;
                }
                Some(diagnostic) => {
                    writeln!(out, "  FAIL  {label}")?;
                    failed.push((label, diagnostic));
                }
            }
        }
    }

    if passed == 0 && failed.is_empty() {
        writeln!(out, "no tests found")?;
        return Ok(true);
    }

    for (name, diagnostic) in &failed {
        writeln!(out, "\n{name}\n{}", render_human(sources, diagnostic))?;
    }

    writeln!(out, "\n{passed} passed, {} failed", failed.len())?;

    Ok(failed.is_empty())
}

/// Runs every test block that the compiled backend can lower and compile.
///
/// Test blocks the backend cannot lower are silently skipped, because the
/// backend compiles a subset of the language on purpose. The blocks that do
/// run must all pass, and the output format matches `run_tests` so the two
/// paths are comparable.
///
/// Each `assert refuses` probe is called and must produce a contract-failure
/// trap. Any other outcome (no trap, or a different kind of trap) is a
/// failure. The body function must not trap at all.
fn run_compiled_tests(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    subject: usize,
) -> io::Result<bool> {
    let mut passed = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut ran = 0usize;

    for checked in &checks[..subject.min(checks.len())] {
        let lowered =
            match deed_mir::lower_with_tests(&checked.module, &checked.resolutions, &checked.types)
            {
                Ok(p) => p,
                Err(_) => continue,
            };

        if lowered.tests.is_empty() {
            continue;
        }

        let compiled = match deed_codegen::compile(&lowered) {
            Ok(m) => m,
            Err(_) => continue,
        };

        writeln!(out, "{}", sources.file(checked.file).name())?;

        for test in &lowered.tests {
            ran += 1;
            let label = &test.name;
            let mut ok = true;

            // The body must complete without trapping.
            if let Err(trap) = deed_codegen::call(&compiled, &test.body, &[]) {
                failed.push((label.clone(), format!("body trapped: {trap}")));
                ok = false;
            }

            // Each `assert refuses` probe must get a contract-failure trap.
            if ok {
                for probe in &test.refuses {
                    match deed_codegen::call(&compiled, probe, &[]) {
                        Err(deed_codegen::Trap::Failed { code, .. })
                            if is_compiled_contract_failure(&code) => {}
                        Ok(_) => {
                            failed.push((
                                label.clone(),
                                "an `assert refuses` expression did not fail".to_string(),
                            ));
                            ok = false;
                            break;
                        }
                        Err(other) => {
                            failed.push((
                                label.clone(),
                                format!("an `assert refuses` probe trapped unexpectedly: {other}"),
                            ));
                            ok = false;
                            break;
                        }
                    }
                }
            }

            if ok {
                passed += 1;
                writeln!(out, "  ok    {label}")?;
            } else {
                writeln!(out, "  FAIL  {label}")?;
            }
        }
    }

    if ran == 0 {
        writeln!(out, "no tests found in the compiled backend")?;
        return Ok(true);
    }

    for (name, reason) in &failed {
        writeln!(out, "\n{name}\n{reason}")?;
    }

    writeln!(out, "\n{passed} passed, {} failed", failed.len())?;

    Ok(failed.is_empty())
}

/// Whether a diagnostic code from the compiled backend counts as a contract
/// failure, which is what `assert refuses` expects.
///
/// Mirrors the interpreter's `is_contract_failure` check.
fn is_compiled_contract_failure(code: &str) -> bool {
    // DEED6002 = PRECONDITION_FAILED
    // DEED6003 = POSTCONDITION_FAILED  (not yet compiled, included for when it is)
    // DEED6004 = REFINEMENT_FAILED     (not yet compiled, included for when it is)
    matches!(code, "DEED6002" | "DEED6003" | "DEED6004")
}

fn report_human(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    obligations: bool,
    timings: bool,
) -> io::Result<()> {
    for checked in checks {
        for diagnostic in &checked.diagnostics {
            writeln!(out, "{}", render_human(sources, diagnostic))?;
        }
    }

    if obligations {
        report_obligations(out, sources, checks)?;
    }
    if timings {
        report_timings(out, checks)?;
    }

    let errors: usize = checks.iter().map(Checked::error_count).sum();
    let warnings: usize = checks.iter().map(Checked::warning_count).sum();
    if errors > 0 || warnings > 0 {
        writeln!(
            out,
            "{}, {}",
            plural(errors, "error"),
            plural(warnings, "warning")
        )?;
    }

    Ok(())
}

/// Wall time per pass, summed over every file.
///
/// P9 says check latency is budgeted. A budget nobody measures is a wish, so
/// this is here to make the claim something a reader can check rather than
/// take on faith. It is wall time on one machine and it says so.
fn report_timings(out: &mut impl Write, checks: &[Checked]) -> io::Result<()> {
    let mut total = deed_driver::Timings::default();
    for checked in checks {
        total.lex += checked.timings.lex;
        total.parse += checked.timings.parse;
        total.resolve += checked.timings.resolve;
        total.typeck += checked.timings.typeck;
        total.effects += checked.timings.effects;
    }

    let whole = total.total();
    writeln!(
        out,
        "timings: {:.2}ms over {}",
        whole.as_secs_f64() * 1000.0,
        plural(checks.len(), "file")
    )?;
    for (name, elapsed) in total.passes() {
        let share = if whole.is_zero() {
            0.0
        } else {
            elapsed.as_secs_f64() / whole.as_secs_f64() * 100.0
        };
        writeln!(
            out,
            "  {name:<8} {:>8.2}ms  {share:>4.0}%",
            elapsed.as_secs_f64() * 1000.0
        )?;
    }

    Ok(())
}

fn report_obligations(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
) -> io::Result<()> {
    let proven: usize = checks.iter().map(|c| c.obligations_at(Tier::Proven)).sum();
    let tested: usize = checks.iter().map(|c| c.obligations_at(Tier::Tested)).sum();
    let guarded: usize = checks.iter().map(|c| c.obligations_at(Tier::Guarded)).sum();

    writeln!(
        out,
        "obligations: {proven} proven, {tested} tested, {guarded} guarded"
    )?;

    for checked in checks {
        let name = sources.file(checked.file).name();
        for ObligationReport {
            tier,
            span,
            subject,
            reason,
        } in &checked.obligations
        {
            let location = sources.file(checked.file).location(span.start);
            let why = match reason {
                Some(reason) => format!("  ({})", reason.text()),
                None => String::new(),
            };
            writeln!(
                out,
                "  {:<8} {name}:{}:{}  {subject}{why}",
                tier.name(),
                location.line,
                location.column
            )?;
        }
    }

    Ok(())
}

fn report_json(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    obligations: bool,
) -> io::Result<()> {
    write!(
        out,
        "{}",
        deed_driver::json_report(sources, checks, obligations)
    )
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Adds the files an import needs, working out where they are from what they
/// are called.
///
/// A module's name says where it lives: a module named `a/b` is at
/// `<root>/a/b.deed`. The root is not configured anywhere and there is no
/// search path. It comes from a file that was named on the command line: take
/// its own module path off the end of its own file path, and what is left is
/// the root. `examples/todo.deed` saying `module examples/todo` puts the root
/// at the directory holding `examples`.
///
/// That rule is not new. Every file in this repository has always been laid
/// out that way, and until now nothing said so out loud, so a program that
/// imported anything could not be run by naming its own file.
///
/// A `deed.manifest` file in a root adds component roots from other source
/// trees. Those roots are searched only after the roots derived from the named
/// files have been asked and have not answered. The manifest cannot change what
/// a module means; it can only say where to look.
///
/// One thing is not under a root: a module that ships inside the compiler.
/// Those are looked at only after every root has been asked. [`deed_driver::
/// resolve_inputs`] is the rule shared with the language server and future
/// dependency resolution: a file called `std/string.deed` sitting under
/// somebody's own root is the one that wins. Nobody should have to know which
/// is which, and the one that is right there is the one they can read.
fn resolve_imports(
    files: &mut Vec<PathBuf>,
    shipped: &mut Vec<&'static str>,
    manifests: &mut Vec<(String, String, Vec<deed_diagnostics::Diagnostic>)>,
) -> io::Result<()> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut component_roots: Vec<PathBuf> = Vec::new();
    let mut known_roots: HashSet<PathBuf> = HashSet::new();
    let mut new_files: Vec<PathBuf> = Vec::new();
    let mut known_files: HashSet<PathBuf> = files.iter().cloned().collect();

    // Read the texts of the initially named files. Two things come from this:
    // the seed texts for resolve_inputs, and the roots that the find closure
    // will use to locate imports on disk.
    let mut seed_texts: Vec<String> = Vec::new();
    for path in files.iter() {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if let Some((module, _)) = deed_driver::imports_of(&text) {
            if let Some(root) = root_of(path, &module) {
                if known_roots.insert(root.clone()) {
                    read_manifest(&root, &mut component_roots, manifests);
                    roots.push(root);
                }
            }
        }
        seed_texts.push(text);
    }

    // The finder: look for each needed module on disk, under every root the
    // seeds have established. When a new file is found, its own root may extend
    // the search for its own imports in the next round.
    let (_, found_shipped) = deed_driver::resolve_inputs(
        seed_texts.iter().map(String::as_str),
        |module: &str| -> Option<(String, String)> {
            let search_roots: Vec<PathBuf> = roots
                .iter()
                .chain(component_roots.iter())
                .cloned()
                .collect();
            for root in search_roots {
                let mut candidate = root;
                for segment in module.split('/') {
                    candidate.push(segment);
                }
                candidate.set_extension("deed");

                if !candidate.is_file() {
                    continue;
                }

                // A file already in the named set or found this run counts as
                // resolved even if we do not add it again.
                if known_files.contains(&candidate) {
                    // Return a non-None to prevent the shipped-module table
                    // from being tried for a module that lives on disk.
                    let text = std::fs::read_to_string(&candidate).unwrap_or_default();
                    return Some((display_path(&candidate), text));
                }

                let Ok(text) = std::fs::read_to_string(&candidate) else {
                    continue;
                };
                if let Some((m, _)) = deed_driver::imports_of(&text) {
                    if let Some(new_root) = root_of(&candidate, &m) {
                        if known_roots.insert(new_root.clone()) {
                            read_manifest(&new_root, &mut component_roots, manifests);
                            roots.push(new_root);
                        }
                    }
                }
                known_files.insert(candidate.clone());
                new_files.push(candidate.clone());
                return Some((display_path(&candidate), text));
            }
            None
        },
    );

    files.extend(new_files);
    *shipped = found_shipped;
    Ok(())
}

/// Reads a `deed.manifest` from `root`, if one exists, adding its component
/// roots to `component_roots` and collecting any parse diagnostics.
///
/// An absent manifest is silently ignored. An unreadable one is silently
/// ignored too; the error message for that is better coming from the OS than
/// from a detour through the diagnostic system.
fn read_manifest(
    root: &Path,
    component_roots: &mut Vec<PathBuf>,
    manifests: &mut Vec<(String, String, Vec<deed_diagnostics::Diagnostic>)>,
) {
    let manifest_path = root.join("deed.manifest");
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return;
    };

    let name = display_path(&manifest_path);

    let mut sources = deed_diagnostics::SourceMap::new();
    let file = sources.add(name.clone(), text.clone());
    let parsed = deed_driver::parse_manifest(file, &text);

    for component in &parsed.components {
        let resolved = if component.path.is_absolute() {
            component.path.clone()
        } else {
            root.join(&component.path)
        };
        if component_roots.contains(&resolved) {
            continue;
        }
        component_roots.push(resolved);
    }

    if parsed.diagnostics.is_empty() {
        return;
    }
    manifests.push((name, text, parsed.diagnostics));
}

/// The directory a module path is relative to, when the file is where its name
/// says it should be.
///
/// `None` when it is not, which is a file that cannot say where anything else
/// lives and so is not asked.
fn root_of(path: &Path, module: &str) -> Option<PathBuf> {
    let mut root = path.to_path_buf();
    root.set_extension("");

    for segment in module.split('/').rev() {
        if root.file_name()?.to_str()? != segment {
            return None;
        }
        root.pop();
    }
    Some(root)
}

/// Forward slashes everywhere, so output does not depend on the platform.
fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let metadata = std::fs::metadata(path)?;

    if metadata.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Nothing good has ever come out of walking into these.
        if name.starts_with('.') || name == "target" {
            continue;
        }

        if entry.file_type()?.is_dir() {
            collect(&entry_path, out)?;
        } else if entry_path.extension().is_some_and(|ext| ext == "deed") {
            out.push(entry_path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "deed-manifest-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn repeated_component_roots_are_added_once() {
        let root = temporary_root("dedup");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("deed.manifest"),
            "component ../shared\ncomponent ../shared\n",
        )
        .unwrap();

        let mut component_roots = Vec::new();
        let mut manifests = Vec::new();
        read_manifest(&root, &mut component_roots, &mut manifests);

        assert_eq!(component_roots, [root.join("../shared")]);
        assert!(manifests.is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn invalid_manifest_diagnostics_are_retained() {
        let root = temporary_root("diagnostic");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("deed.manifest"), "component\n").unwrap();

        let mut component_roots = Vec::new();
        let mut manifests = Vec::new();
        read_manifest(&root, &mut component_roots, &mut manifests);

        assert!(component_roots.is_empty());
        assert_eq!(manifests.len(), 1);
        assert_eq!(
            manifests[0].2[0].code,
            deed_driver::codes::MISSING_COMPONENT_PATH
        );
        std::fs::remove_dir_all(root).ok();
    }
}

#[cfg(test)]
mod compiled_tests {
    use super::*;
    use deed_codegen::wasm::{Func, FuncType, Ins, ValType};

    #[test]
    fn compiled_failure_codes_keep_their_diagnostic_labels() {
        let mut sources = SourceMap::new();
        let file = sources.add("test.deed", "0123456789");
        let span = deed_diagnostics::Span::new(2, 5);

        for (code, label) in [
            (deed_mir::codes::ASSERTION_FAILED, "evaluated to false"),
            (deed_mir::codes::PRECONDITION_FAILED, "precondition not met"),
            (deed_mir::codes::NOT_RUNNABLE, "not runnable"),
        ] {
            let trap = deed_codegen::Trap::Failed {
                code: code.to_string(),
                message: "stopped".to_string(),
                span: Some(span),
                blame_caller: false,
            };
            let diagnostic = compiled_diagnostic(file, &trap).expect("known code");
            assert_eq!(diagnostic.code, code);
            assert_eq!(diagnostic.primary.span, span);
            assert_eq!(diagnostic.primary.message, label);
        }
    }

    #[test]
    fn compiled_arguments_use_the_named_exports_signature() {
        let mut module = deed_codegen::Module::new();
        let other_ty = module.intern_type(FuncType {
            params: vec![ValType::I32],
            results: vec![],
        });
        let other = module.add_func(Func {
            type_index: other_ty,
            locals: vec![],
            body: vec![Ins::I32Const(0)],
        });
        module.export("other", other);

        let main_ty = module.intern_type(FuncType {
            params: vec![ValType::I64, ValType::I32],
            results: vec![],
        });
        let main = module.add_func(Func {
            type_index: main_ty,
            locals: vec![],
            body: vec![Ins::I64Const(0)],
        });
        module.export("main", main);

        assert_eq!(
            compiled_args(&module, "main"),
            Some(vec![
                deed_codegen::Value::I64(0),
                deed_codegen::Value::I32(0)
            ])
        );
        assert!(compiled_args(&module, "missing").is_none());
    }
}
