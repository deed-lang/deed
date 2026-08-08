//! `deed new`, exercised by running the binary and then running the binary
//! again on what it wrote.
//!
//! The scaffold is the first Deed anybody sees, and it is written in a Rust
//! string literal where nothing in the normal corpus ratchets reaches it. So
//! the claim is held here end to end: what `deed new` writes has to check
//! without a word of complaint, pass its own tests, run, and already be in the
//! form `deed fmt` would put it in. A scaffold that rots fails one of these by
//! name.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEED: &str = env!("CARGO_BIN_EXE_deed");

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-new-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(DEED)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the deed binary should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("should exit normally")
}

#[test]
fn what_it_writes_checks_tests_runs_and_is_already_formatted() {
    let scratch = Scratch::new("green");
    let made = run_in(scratch.path(), &["new", "greeter"]);
    assert_eq!(code(&made), 0, "{}", stderr(&made));

    let project = scratch.path().join("greeter");

    // Silence is the whole claim: not one warning either, or a new project
    // starts by teaching somebody to ignore output.
    let checked = run_in(&project, &["check", "."]);
    assert_eq!(code(&checked), 0, "{}", stderr(&checked));
    assert_eq!(stdout(&checked), "", "{}", stdout(&checked));
    assert_eq!(stderr(&checked), "", "{}", stderr(&checked));

    let tested = run_in(&project, &["test", "."]);
    assert_eq!(code(&tested), 0, "{}", stderr(&tested));
    assert!(stdout(&tested).contains("0 failed"), "{}", stdout(&tested));
    assert!(
        !stdout(&tested).contains("0 passed"),
        "a scaffold with no tests would pass this file's other assertions: {}",
        stdout(&tested)
    );

    let ran = run_in(&project, &["run", "main.deed"]);
    assert_eq!(code(&ran), 0, "{}", stderr(&ran));
    assert!(!stdout(&ran).is_empty(), "the program printed nothing");

    let formatted = run_in(&project, &["fmt", "--check", "."]);
    assert_eq!(
        code(&formatted),
        0,
        "the scaffold is not the canonical form: {}",
        stdout(&formatted)
    );
}

#[test]
fn it_says_which_files_it_wrote() {
    let scratch = Scratch::new("listing");
    let made = run_in(scratch.path(), &["new", "ledger"]);
    let said = stdout(&made);

    for file in ["ledger.deed", "main.deed"] {
        let path = scratch.path().join("ledger").join(file);
        assert!(path.is_file(), "`{file}` was not written");
        assert!(said.contains(file), "`{file}` was not reported: {said}");
    }
}

#[test]
fn the_program_half_imports_the_library_half() {
    let scratch = Scratch::new("import");
    run_in(scratch.path(), &["new", "ledger"]);

    let program = std::fs::read_to_string(scratch.path().join("ledger").join("main.deed")).unwrap();
    assert!(program.contains("use ledger."), "{program}");
}

#[test]
fn it_writes_no_manifest() {
    let scratch = Scratch::new("manifest");
    run_in(scratch.path(), &["new", "ledger"]);

    // A manifest here says where code outside your tree lives. A new project
    // has none, and a file of commented-out examples is text nothing reads.
    assert!(
        !scratch.path().join("ledger").join("deed.manifest").exists(),
        "a new project has no dependency to name"
    );
}

#[test]
fn it_refuses_a_directory_that_is_already_there() {
    let scratch = Scratch::new("occupied");
    std::fs::create_dir(scratch.path().join("greeter")).unwrap();
    std::fs::write(
        scratch.path().join("greeter").join("mine.deed"),
        "module m\n",
    )
    .unwrap();

    let made = run_in(scratch.path(), &["new", "greeter"]);
    assert_eq!(code(&made), 2, "{}", stdout(&made));
    assert!(stderr(&made).contains("already there"), "{}", stderr(&made));

    // The point of refusing is the file that was already in there.
    let kept = std::fs::read_to_string(scratch.path().join("greeter").join("mine.deed")).unwrap();
    assert_eq!(kept, "module m\n");
}

#[test]
fn it_refuses_a_name_the_language_could_not_hold() {
    let scratch = Scratch::new("keyword");
    let made = run_in(scratch.path(), &["new", "effect"]);
    assert_eq!(code(&made), 2, "{}", stdout(&made));
    assert!(stderr(&made).contains("keyword"), "{}", stderr(&made));
    assert!(
        !scratch.path().join("effect").exists(),
        "a refused name left a directory behind"
    );
}

#[test]
fn a_capital_gets_the_name_that_would_have_worked() {
    let scratch = Scratch::new("capital");
    let made = run_in(scratch.path(), &["new", "Greeter"]);
    assert_eq!(code(&made), 2, "{}", stdout(&made));
    assert!(stderr(&made).contains("`greeter`"), "{}", stderr(&made));
}

#[test]
fn it_takes_one_name_and_no_options() {
    let scratch = Scratch::new("arity");

    let none = run_in(scratch.path(), &["new"]);
    assert_eq!(code(&none), 2);
    assert!(stderr(&none).contains("needs a name"), "{}", stderr(&none));

    let two = run_in(scratch.path(), &["new", "a", "b"]);
    assert_eq!(code(&two), 2);
    assert!(stderr(&two).contains("one name"), "{}", stderr(&two));
    assert!(!scratch.path().join("a").exists(), "it wrote one anyway");
}
