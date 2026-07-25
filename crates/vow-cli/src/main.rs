//! The `vow` command line tool.

mod args;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vow_diagnostics::{SourceMap, render_human, render_json};
use vow_driver::{Checked, ObligationReport};
use vow_typeck::Tier;

use crate::args::{CheckArgs, Command, Format, USAGE};

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

    let mut sources = SourceMap::new();
    let mut checks = Vec::new();

    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: {}: {error}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };
        let file = sources.add(display_path(path), text);
        checks.push(vow_driver::check(&sources, file));
    }

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
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
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
    let guarded: usize = checks.iter().map(|c| c.obligations_at(Tier::Guarded)).sum();

    writeln!(
        out,
        "obligations: {proven} proven, 0 tested, {guarded} guarded"
    )?;
    // Saying "0 tested" without saying why would imply the tier is empty by
    // choice rather than unbuilt.
    writeln!(
        out,
        "  the tested tier needs property test generation, which does not exist yet"
    )?;

    for checked in checks {
        let name = sources.file(checked.file).name();
        for ObligationReport {
            tier,
            span,
            refinement,
        } in &checked.obligations
        {
            let location = sources.file(checked.file).location(span.start);
            writeln!(
                out,
                "  {:<8} {name}:{}:{}  {refinement}",
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
                refinement,
            } in &checked.obligations
            {
                let location = file.location(span.start);
                writeln!(
                    out,
                    "{{\"kind\":\"obligation\",\"tier\":\"{}\",\"file\":\"{}\",\"line\":{},\"column\":{},\"refinement\":\"{}\"}}",
                    tier_name(*tier),
                    file.name(),
                    location.line,
                    location.column,
                    refinement
                )?;
            }
        }
    }

    Ok(())
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Proven => "proven",
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
