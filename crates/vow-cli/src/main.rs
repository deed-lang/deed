//! The `vow` command line tool.

mod args;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vow_diagnostics::{SourceMap, render_human, render_json};
use vow_driver::{Checked, ObligationReport};
use vow_interp::{Program, PropertyConfig};
use vow_typeck::Tier;

use crate::args::{CheckArgs, Command, Format, Mode, USAGE};

/// Something went wrong with the invocation rather than with the code.
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
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
            println!("vow {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Check(check) => run_check(check),
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
        eprintln!("error: no `.vow` files found");
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

    // Every file at once, so a `use` has something to point at. Checking them
    // one at a time would mean an import could never resolve, which is how it
    // used to work and why nothing crossing a module boundary was checked.
    let checks = vow_driver::check_all(&sources, &ids);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let result = match args.format {
        Format::Human => report_human(&mut out, &sources, &checks, args.obligations),
        Format::Json => report_json(&mut out, &sources, &checks, args.obligations),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        return ExitCode::from(EXIT_USAGE);
    }

    let errors: usize = checks.iter().map(Checked::error_count).sum();
    if errors > 0 {
        return ExitCode::FAILURE;
    }

    if args.mode == Mode::Test {
        // Running code that does not check would be answering a question
        // nobody asked, and the failure would be about the wrong thing.
        match run_tests(&mut out, &sources, &checks) {
            Ok(true) => {}
            Ok(false) => return ExitCode::FAILURE,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    if args.mode == Mode::Run {
        match run_main(&mut out, &sources, &checks, args.dir.as_deref()) {
            Ok(Some(true)) => {}
            Ok(Some(false)) => return ExitCode::FAILURE,
            Ok(None) => return ExitCode::from(EXIT_USAGE),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    ExitCode::SUCCESS
}

/// Rewrites files into canonical form, or reports which are not.
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
        let formatted = match vow_fmt::format(file, &text) {
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
        program.add(checked.file, &checked.module, &checked.resolutions);
    }
    program
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
        let fixed = vow_driver::fix::fix(&original, |text| diagnose(&name, text));

        if !fixed.changed() {
            continue;
        }

        // A fix that leaves more errors than it found is a wrong fix, not a
        // partial one, so the file is left alone and the run says so.
        let after = diagnose(&name, &fixed.source);
        if vow_driver::fix::error_count(&after) > vow_driver::fix::error_count(&before) {
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
        println!("{name}: still changing after several rounds, run `vow fix` again");
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
fn diagnose(name: &str, text: &str) -> Vec<vow_diagnostics::Diagnostic> {
    let mut sources = SourceMap::new();
    let file = sources.add(name.to_string(), text.to_string());
    vow_driver::check(&sources, file).diagnostics
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
    dir: Option<&Path>,
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
    for checked in checks {
        if let Some(run) = vow_interp::run_main(&program, checked.file, &root) {
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
    match run.result {
        Ok(_) => Ok(Some(true)),
        Err(failure) => {
            writeln!(out, "{}", render_human(sources, &failure))?;
            Ok(Some(false))
        }
    }
}

/// Runs every test in every checked file. Returns whether they all passed.
fn run_tests(out: &mut impl Write, sources: &SourceMap, checks: &[Checked]) -> io::Result<bool> {
    let mut passed = 0usize;
    let mut failed = Vec::new();
    let program = program_of(checks);

    for checked in checks {
        let outcomes = vow_interp::run_tests(&program, checked.file);
        let properties = vow_interp::run_properties(
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

fn report_human(
    out: &mut impl Write,
    sources: &SourceMap,
    checks: &[Checked],
    obligations: bool,
) -> io::Result<()> {
    for checked in checks {
        for diagnostic in &checked.diagnostics {
            writeln!(out, "{}", render_human(sources, diagnostic))?;
        }
    }

    if obligations {
        report_obligations(out, sources, checks)?;
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
        } in &checked.obligations
        {
            let location = sources.file(checked.file).location(span.start);
            writeln!(
                out,
                "  {:<8} {name}:{}:{}  {subject}",
                tier_name(*tier),
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
    for checked in checks {
        for diagnostic in &checked.diagnostics {
            writeln!(
                out,
                "{{\"kind\":\"diagnostic\",\"diagnostic\":{}}}",
                render_json(sources, diagnostic)
            )?;
        }
    }

    if obligations {
        for checked in checks {
            let file = sources.file(checked.file);
            for ObligationReport {
                tier,
                span,
                subject,
            } in &checked.obligations
            {
                let location = file.location(span.start);
                writeln!(
                    out,
                    "{{\"kind\":\"obligation\",\"tier\":\"{}\",\"file\":\"{}\",\"line\":{},\"column\":{},\"subject\":\"{}\"}}",
                    tier_name(*tier),
                    file.name(),
                    location.line,
                    location.column,
                    subject
                )?;
            }
        }
    }

    Ok(())
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Proven => "proven",
        Tier::Tested => "tested",
        Tier::Guarded => "guarded",
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
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
        } else if entry_path.extension().is_some_and(|ext| ext == "vow") {
            out.push(entry_path);
        }
    }

    Ok(())
}
