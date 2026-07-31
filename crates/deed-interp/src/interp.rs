//! A tree walking interpreter.
//!
//! Five passes decide what a program is allowed to do; this is the first thing
//! that does it. It exists mostly for one reason: two of the three verification
//! tiers in `design/02-syntax.md` need a runtime. `Guarded` obligations were
//! being recorded and never checked, which is exactly the quiet degradation the
//! design says must not happen.
//!
//! Runtime failures are [`Diagnostic`]s, not panics and not strings. A program
//! that fails while running is not a different kind of problem from one that
//! fails while being checked, and P7 does not stop applying because the
//! compiler finished.
//!
//! What this refuses falls into three kinds, and telling them apart is most of
//! what a runtime message has to do. Most of them are shapes `deed check`
//! turns down, so a run that meets one has been handed a file nobody checked
//! or has found a hole in the check. A few are the interpreter's own unfinished
//! work, on programs the checker accepts. And a few are neither: a contract
//! broken, arithmetic with no answer, a directory the run was never given.
//! Every message anything can reach is read by a test, and
//! `crates/deed-interp/tests/messages.rs` is where the reading is written
//! down, along with the argument for keeping the two arms nothing can reach.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use deed_ast::{
    BinaryOp, Block, Ensures, Expr, FieldInit, FnDecl, HandlerDecl, Ident, Item, Module, Outcome,
    Param, Pattern, Stmt, UnaryOp,
};
use deed_diagnostics::{ByNumber, Diagnostic, FileId, Span};
use deed_resolve::{DefId, DefKind, ExportKind, Resolutions};

use crate::codes;
use crate::sandbox;
use crate::value::{Capability, ClosureValue, Fields, Frame, Value, VariantValue};

/// How deep a chain of calls may go before the interpreter gives up.
///
/// Not a language rule and not a promise about how much recursion is
/// reasonable. It is the number that decides whether an unbounded recursion
/// produces a diagnostic or takes the process down with it, and the second is
/// not an acceptable answer for a tool people point at code they have not read.
///
/// It is low, and deliberately. The interpreter walks the tree by recursion, so
/// one Deed frame costs several host frames, and this is a library that does not
/// get to choose the stack it is handed. A limit that only holds when the
/// caller was generous is not a limit. `deed` itself runs on a thread with far
/// more room than this needs, which is the margin rather than the budget.
const MAX_DEPTH: usize = 128;

/// How one `test` block went.
pub struct TestOutcome {
    pub name: String,
    pub span: Span,
    /// `None` when it passed.
    pub failure: Option<Diagnostic>,
}

impl TestOutcome {
    pub fn passed(&self) -> bool {
        self.failure.is_none()
    }
}

/// Every module the interpreter has the code of.
///
/// A call that goes through an import has to run in the module the body was
/// written in, using that module's resolutions. Reading the callee's names out
/// of the caller's scope would be a class of bug that does not announce itself,
/// so the current module is part of the interpreter's state rather than
/// something a call site remembers to pass.
#[derive(Default)]
pub struct Program<'a> {
    entries: Vec<Entry<'a>>,
}

struct Entry<'a> {
    path: Rc<str>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    guards: Guards,
    rows: DeclaredRows,
}

/// Where the type checker gave up, and on what.
///
/// The key is the expression that has to satisfy the refinement. It comes from
/// the checker rather than being worked out again here, because the two passes
/// disagreeing about what `Guarded` means is exactly how a tier turns into a
/// lie: the checker used to say "so it becomes a runtime check" at places the
/// interpreter never checked.
pub type Guards = HashMap<Span, Guard>;

/// What has to be checked at one span.
#[derive(Clone, Copy, Debug)]
pub struct Guard {
    /// The refinement the value has to satisfy.
    pub refinement: DefId,
    /// Whether the value is the number inside the `ok` rather than the
    /// expression itself.
    ///
    /// A `Result` that came back from a call has nothing naming its payload,
    /// so the obligation lands on the whole expression. Running the predicate
    /// against the `Result` fails whatever is inside it, which turned a check
    /// that should have passed into "the interpreter cannot run `>` on a
    /// Result and an Int".
    pub inside_ok: bool,
}

/// One entry of a row a function declared, as the module that wrote it sees it.
///
/// The same shape as the effect checker's own entry, kept separately so that
/// running a program does not depend on the pass that checked it. What crosses
/// is data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RowItem {
    pub effect: DefId,
    /// `None` means every operation of the effect.
    pub operation: Option<String>,
}

/// What each function in a module declared it performs, by where its name was
/// written.
///
/// A span rather than a definition, because a handler operation has neither a
/// definition nor a name of its own outside the handler, and a handler
/// operation is where an effect is actually implemented.
///
/// The rows are the argument this language is making, and until this existed
/// the only thing that ever looked at one was the pass that produced it. With
/// it, every `test` block in every file is a check on that pass: the program
/// runs, and an effect performed inside a function that did not declare it is
/// reported against the compiler rather than against the program.
///
/// Not optional, for the same reason `Guards` is not. A caller that could
/// leave it out would be one that could turn the check off by forgetting.
pub type DeclaredRows = HashMap<Span, Vec<RowItem>>;

impl<'a> Program<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module. A file with no `module` line cannot be imported, so it
    /// is registered under its own file name and nothing can reach it.
    ///
    /// `guards` is not optional on purpose. A caller that could leave it out
    /// would be a caller that could turn every runtime check off by forgetting
    /// something, silently, and the warning would still be printed. `rows` is
    /// not optional for the same reason: see [`DeclaredRows`].
    pub fn add(
        &mut self,
        file: FileId,
        module: &'a Module,
        resolutions: &'a Resolutions,
        guards: Guards,
        rows: DeclaredRows,
    ) {
        let path = match &module.name {
            Some(name) => name.to_string_path(),
            None => format!("<file {}>", file.index()),
        };
        self.entries.push(Entry {
            path: Rc::from(path.as_str()),
            file,
            module,
            resolutions,
            guards,
            rows,
        });
    }

    fn index_of(&self, file: FileId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.file == file)
    }

    fn module(&self, file: FileId) -> Option<&'a Module> {
        self.entries
            .iter()
            .find(|entry| entry.file == file)
            .map(|entry| entry.module)
    }
}

/// Runs every `test` block in one of the program's modules.
pub fn run_tests(program: &Program, file: FileId) -> Vec<TestOutcome> {
    let Some(module) = program.module(file) else {
        return Vec::new();
    };

    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Test(test) => Some(test),
            _ => None,
        })
        .map(|test| {
            let mut interp = Interp::new(program, file);
            let failure = match interp.eval_block(&test.body) {
                Ok(_) | Err(Signal::Return(_)) => None,
                Err(Signal::Fail(diagnostic)) => Some(*diagnostic),
            };
            TestOutcome {
                name: test.name.clone(),
                span: test.name_span,
                failure,
            }
        })
        .collect()
}

/// What running a program produced.
pub struct Run {
    /// Lines written through a `Console`, in order.
    pub output: Vec<String>,
    /// `Err` when the program failed, which includes a contract it broke.
    pub result: Result<Value, Box<Diagnostic>>,
    /// Runtime profile, when one was requested.
    pub profile: Option<RuntimeProfile>,
}

/// How one function contributed to runtime.
#[derive(Clone, Debug)]
pub struct FunctionProfile {
    pub module: String,
    pub function: String,
    pub calls: u64,
    pub contract_checks: u64,
    pub handler_calls: u64,
    pub total: Duration,
    pub contract: Duration,
    pub handler: Duration,
}

/// Runtime profile for one `main` run.
#[derive(Clone, Debug)]
pub struct RuntimeProfile {
    pub total: Duration,
    pub functions: Vec<FunctionProfile>,
}

/// Runs `main`, handing it the one `System` capability that exists.
///
/// There is no other way to obtain one, which is what makes reading `main`
/// enough to know the whole attack surface of a program.
///
/// `root` is the directory `sys.files` is rooted at, and the program can reach
/// nothing outside it. The caller supplies it rather than the runtime picking
/// one, because how much of the filesystem a program gets is a decision and
/// decisions belong at the call site. `arguments` arrives the same way and for
/// the same reason: the runtime does not read the process's own command line,
/// so nothing about how the compiler was invoked can leak into the program it
/// is running.
pub fn run_main(program: &Program, file: FileId, root: &Path, arguments: &[String]) -> Option<Run> {
    run_main_with_profile(program, file, root, arguments, false)
}

/// Runs `main` and records where runtime went.
pub fn run_main_profiled(
    program: &Program,
    file: FileId,
    root: &Path,
    arguments: &[String],
) -> Option<Run> {
    run_main_with_profile(program, file, root, arguments, true)
}

fn run_main_with_profile(
    program: &Program,
    file: FileId,
    root: &Path,
    arguments: &[String],
    profile: bool,
) -> Option<Run> {
    let main = program
        .module(file)?
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.sig.name.name == "main" => Some(function),
            _ => None,
        })?;

    let mut interp = Interp::new(program, file);
    if profile {
        interp.profile = Some(ProfileState::new());
    }
    interp.root = sandbox::root(root).ok().map(Rc::from);
    interp.arguments = arguments
        .iter()
        .map(|argument| Rc::from(&**argument))
        .collect();

    let span = main.sig.name.span;
    let args = main
        .sig
        .params
        .iter()
        .map(|_| (Value::Capability(Capability::System), span))
        .collect();

    let result = interp.call_from_outside(main, args, span);
    let profile = interp.runtime_profile();
    Some(Run {
        output: interp.output.clone(),
        result,
        profile,
    })
}

/// Non-local control flow.
///
/// `Return` is ordinary, `Fail` ends the program. The one thing that catches
/// is `assert refuses`, and it catches contract failures and nothing else.
enum Signal {
    Return(Value),
    Fail(Box<Diagnostic>),
}

/// Whether a runtime failure is a contract turning a value down.
///
/// The three ways a signature can refuse: what a caller had to guarantee, what
/// a function had to guarantee back, and what a refined type admits. Everything
/// else that can end a run is a program going wrong rather than a contract
/// doing its job, which is the line `assert refuses` is drawn along.
fn is_contract_failure(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code,
        codes::PRECONDITION_FAILED | codes::POSTCONDITION_FAILED | codes::REFINEMENT_FAILED
    )
}

/// Whether a runtime variant is the one a pattern named.
fn variant_is(variant: &VariantValue, id: &(Rc<str>, String)) -> bool {
    variant.origin == id.0 && variant.name == id.1
}

/// How a unary operator was written.
///
/// Here rather than on [`UnaryOp`], because the one thing that needs it is a
/// diagnostic and `BinaryOp::as_str` earns its place in the syntax tree by
/// being what the formatter prints.
fn unary_op_as_str(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Not => "!",
    }
}

/// Walks one module's items once, so lookups during a run are by definition.
fn index_module<'a>(entry: &Entry<'a>) -> Code<'a> {
    let resolutions = entry.resolutions;
    let def_of = |ident: &Ident| resolutions.resolution(ident.span);

    let mut code = Code {
        path: Rc::clone(&entry.path),
        file: entry.file,
        resolutions,
        functions: HashMap::default(),
        handler_decls: HashMap::default(),
        refinements: HashMap::default(),
        subjects: HashMap::default(),
        guards: entry.guards.clone(),
        rows: entry.rows.clone(),
        state_names: HashMap::default(),
        variant_names: HashMap::default(),
        plans: HashMap::default(),
    };

    for item in &entry.module.items {
        match item {
            Item::Function(function) => {
                if let Some(def) = def_of(&function.sig.name) {
                    code.functions.insert(def, function);
                }
            }
            Item::Handler(handler) => {
                if let Some(def) = def_of(&handler.name) {
                    code.handler_decls.insert(def, handler);
                }
                for field in &handler.state {
                    if let Some(def) = def_of(&field.name) {
                        code.state_names.insert(def, field.name.name.clone());
                    }
                }
            }
            Item::Choice(choice) => {
                for variant in &choice.variants {
                    if let Some(def) = def_of(&variant.name) {
                        code.variant_names.insert(def, variant.name.name.clone());
                    }
                }
            }
            Item::TypeAlias(alias) => {
                if let (Some(def), Some(predicate)) =
                    (def_of(&alias.name), alias.refinement.as_ref())
                {
                    code.refinements.insert(def, predicate);
                    // `value` is the only name the language introduces on its
                    // own. Resolution gives it a definition whose span is the
                    // alias name, which is what makes it findable from here
                    // without matching on the word.
                    if let Some((subject, _)) = resolutions.defs().find(|(_, data)| {
                        data.kind == DefKind::Local
                            && data.name == "value"
                            && data.span == alias.name.span
                    }) {
                        code.subjects.insert(def, subject);
                    }
                }
            }
            _ => {}
        }
    }

    code
}

type Eval<T> = Result<T, Signal>;

/// Which module declared something, and what it is called there.
///
/// Used for anything that has to be the same thing seen from two modules. A
/// `DefId` cannot do this: it is an index into one module's resolution table,
/// so two unrelated declarations can share a number and the same declaration
/// reached two ways cannot.
type Origin = (Rc<str>, String);

/// A handler installed by a `with` block.
struct Instance {
    handler: DefId,
    /// Which module the handler's operations are written in.
    ///
    /// A `with` block can name a handler from anywhere, and its bodies read
    /// that module's names, not the ones where the block was written.
    module: usize,
    effect: Origin,
    state: Fields,
}

/// One module's code, indexed the way the interpreter needs it.
struct Code<'a> {
    path: Rc<str>,
    file: FileId,
    resolutions: &'a Resolutions,
    functions: HashMap<DefId, &'a FnDecl, ByNumber>,
    handler_decls: HashMap<DefId, &'a HandlerDecl, ByNumber>,
    /// Type alias definition to the predicate it refines by.
    refinements: HashMap<DefId, &'a Expr, ByNumber>,
    /// Type alias definition to the `value` its predicate talks about.
    subjects: HashMap<DefId, DefId, ByNumber>,
    /// Expressions the checker could not settle, and what they have to satisfy.
    guards: Guards,
    /// What each function in this module declared it performs.
    rows: DeclaredRows,
    /// Handler state definition to the field name it stands for.
    state_names: HashMap<DefId, String, ByNumber>,
    variant_names: HashMap<DefId, String, ByNumber>,
    /// What a call to each function here needs, keyed by where the name was
    /// written. Filled in as functions are called. A handler operation has no
    /// definition of its own but does have a span, which is the same reason
    /// `rows` is keyed that way.
    plans: HashMap<Span, Rc<CallPlan>, ByNumber>,
}

/// What one active call promised, so that what it does can be held to it.
///
/// `handled` is how many handlers were installed when the call started. An
/// effect answered by a handler installed after that was discharged by a `with`
/// block inside this call, and a `with` block is how an effect stops being the
/// caller's business. So only effects answered from further out are this
/// frame's to declare, which is the same rule the checker uses and the reason
/// a `with` block does not have to be mentioned in a row.
///
/// `handler` is set when the call is a handler operation, and says which
/// instance it belongs to. What such an operation performs is charged to
/// whoever installed the handler rather than to whoever happened to be on the
/// stack in between, because installing it is the decision that caused it.
/// Without this a function calling `Log.note` would owe whatever the handler
/// the caller chose does, which it cannot know and did not decide.
///
/// `promise` is `None` when the declaration's row holds a row variable. That
/// stands for whatever the caller passed, so the declaration alone does not say
/// what this call may perform, and the caller's own frame is where the question
/// has an answer. Such a frame is not held to anything and does not stop the
/// frames around it being held to theirs.
struct RowFrame {
    handled: usize,
    handler: Option<usize>,
    promise: Option<Rc<Promise>>,
}

struct Promise {
    name: String,
    allowed: Vec<(Origin, Option<String>)>,
}

/// What every call to one function needs before its body can run.
///
/// All of it is a property of the declaration rather than of the call, and it
/// used to be worked out again on every call: a resolution lookup per
/// parameter to find out what the parameter binds, the promised row rebuilt
/// from the declaration's span, and two maps captured on entry for `old(...)`
/// and `unchanged(...)` whether or not anything could read them. That made a
/// call cost about four turns of a `for` on a function with no contract at
/// all, which is measured by `examples/interpreting.rs`.
///
/// Worked out the first time the function is called rather than for every
/// declaration up front, for the same reason closure bodies are: most of a
/// program is not reached by any given run.
struct CallPlan {
    /// What each parameter binds, in order. `None` for a parameter that did
    /// not resolve, which is a file with an error in it being run anyway.
    params: Vec<Option<DefId>>,
    /// The row the declaration promised, or `None` when there is nothing to
    /// hold a call to. See [`RowFrame`].
    promise: Option<Rc<Promise>>,
    /// Whether anything can read what a call captures on entry. That is what
    /// an `ensures` clause is for, and `old` and `unchanged` are refused
    /// outside one, so a function with no `ensures` has nothing to capture.
    captures: bool,
}

/// A closure expression, kept where a [`Value`] can point at it.
///
/// Filled in as closures are evaluated rather than by walking every module up
/// front, because most of them are never reached. One entry per closure
/// expression, not one per evaluation: a closure literal written inside a
/// `for` body or a function that calls itself is evaluated again on every turn,
/// and a table that grew each time would grow with what the program does rather
/// than with how much of it there is.
struct Closure<'a> {
    module: usize,
    params: &'a [Param],
    body: &'a Expr,
}

pub(crate) struct Interp<'a> {
    modules: Vec<Code<'a>>,
    by_path: HashMap<Rc<str>, usize>,
    /// Which module the running frame belongs to.
    current: usize,

    /// One per active call. Bindings are keyed by definition, which resolution
    /// already made unique, so blocks need no scopes of their own.
    ///
    /// Keyed by number rather than by SipHash, which is the other half of what
    /// reading a name costs. See `deed_diagnostics::hashing`.
    frames: Vec<Frame>,
    /// What each active call is allowed to perform, and what was already
    /// handled when it started. See [`Interp::check_row`].
    rows: Vec<RowFrame>,
    /// How deep inside a contract clause the running code is.
    ///
    /// Contracts do not contribute to a row, so performing an effect while
    /// evaluating one is not something a signature has to admit to.
    in_contract: usize,
    handlers: Vec<Instance>,
    /// Which handler instance the running operation belongs to, if any.
    inside_handler: Vec<usize>,

    /// Values of `old(...)` captured on entry to the running call.
    olds: Vec<HashMap<Span, Value>>,
    /// Handler state captured on entry, for `unchanged(...)`.
    entry_states: Vec<HashMap<Origin, Fields>>,

    /// Bodies of the closures evaluated so far. See [`Closure`].
    closures: Vec<Closure<'a>>,
    /// Where each of those went, so evaluating the same literal twice finds
    /// the entry it made the first time. A module and a span name exactly one
    /// expression in the program.
    closure_at: HashMap<(usize, Span), usize, ByNumber>,

    /// Lines written through a `Console`. Collected rather than printed so the
    /// caller decides what to do with them, and so a test can read them.
    output: Vec<String>,
    /// A monotonic clock. Wall clock time would make every run different, and
    /// P8 says the default is deterministic.
    ticks: i64,
    /// Where `sys.files` is rooted. `None` when nothing granted one, which is
    /// the case for `test` blocks: a test that could reach the filesystem
    /// would be a test of the filesystem.
    root: Option<Rc<Path>>,
    /// What the program was invoked with, for `Io.args`.
    ///
    /// Empty for a `test` block, for the same reason as `root`: a test whose
    /// answer depended on how the test runner was invoked would be a test of
    /// the test runner.
    arguments: Vec<Rc<str>>,
    profile: Option<ProfileState>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ProfileKey {
    module: String,
    function: String,
}

#[derive(Default)]
struct FunctionTotals {
    calls: u64,
    contract_checks: u64,
    handler_calls: u64,
    total: Duration,
    contract: Duration,
    handler: Duration,
}

struct ProfileState {
    started: Instant,
    stack: Vec<ProfileKey>,
    functions: HashMap<ProfileKey, FunctionTotals>,
}

impl ProfileState {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            stack: Vec::new(),
            functions: HashMap::new(),
        }
    }
}

impl<'a> Interp<'a> {
    fn new(program: &Program<'a>, file: FileId) -> Self {
        let mut modules = Vec::new();
        let mut by_path = HashMap::new();

        for entry in &program.entries {
            by_path.insert(Rc::clone(&entry.path), modules.len());
            modules.push(index_module(entry));
        }

        Interp {
            current: program.index_of(file).unwrap_or(0),
            modules,
            by_path,
            frames: vec![Frame::default()],
            rows: Vec::new(),
            in_contract: 0,
            handlers: Vec::new(),
            inside_handler: Vec::new(),
            olds: Vec::new(),
            entry_states: Vec::new(),
            closures: Vec::new(),
            closure_at: HashMap::default(),
            output: Vec::new(),
            ticks: 0,
            root: None,
            arguments: Vec::new(),
            profile: None,
        }
    }

    fn runtime_profile(&self) -> Option<RuntimeProfile> {
        let profile = self.profile.as_ref()?;
        let functions = profile
            .functions
            .iter()
            .map(|(key, totals)| FunctionProfile {
                module: key.module.clone(),
                function: key.function.clone(),
                calls: totals.calls,
                contract_checks: totals.contract_checks,
                handler_calls: totals.handler_calls,
                total: totals.total,
                contract: totals.contract,
                handler: totals.handler,
            })
            .collect();
        Some(RuntimeProfile {
            total: profile.started.elapsed(),
            functions,
        })
    }

    fn enter_profiled_call(&mut self, function: &FnDecl) -> Option<(ProfileKey, Instant)> {
        let key = ProfileKey {
            module: self.here().to_string(),
            function: function.sig.name.name.clone(),
        };
        let profile = self.profile.as_mut()?;
        profile.stack.push(key.clone());
        Some((key, Instant::now()))
    }

    fn finish_profiled_call(&mut self, started: Option<(ProfileKey, Instant)>, is_handler: bool) {
        let Some((key, start)) = started else {
            return;
        };
        let Some(profile) = self.profile.as_mut() else {
            return;
        };
        let elapsed = start.elapsed();

        let totals = profile.functions.entry(key.clone()).or_default();
        totals.calls += 1;
        totals.total += elapsed;
        if is_handler {
            totals.handler_calls += 1;
            totals.handler += elapsed;
        }

        profile.stack.pop();
    }

    fn add_contract_time(&mut self, elapsed: Duration) {
        let Some(profile) = self.profile.as_mut() else {
            return;
        };
        let Some(key) = profile.stack.last().cloned() else {
            return;
        };
        let totals = profile.functions.entry(key).or_default();
        totals.contract_checks += 1;
        totals.contract += elapsed;
    }

    // -- the module the running frame belongs to ---------------------------

    fn code(&self) -> &Code<'a> {
        &self.modules[self.current]
    }

    fn resolutions(&self) -> &'a Resolutions {
        self.modules[self.current].resolutions
    }

    fn file(&self) -> FileId {
        self.modules[self.current].file
    }

    fn here(&self) -> Rc<str> {
        Rc::clone(&self.modules[self.current].path)
    }

    fn function(&self, def: DefId) -> Option<&'a FnDecl> {
        self.code().functions.get(&def).copied()
    }

    fn handler_decl(&self, def: DefId) -> Option<&'a HandlerDecl> {
        self.code().handler_decls.get(&def).copied()
    }

    fn refinement(&self, def: DefId) -> Option<&'a Expr> {
        self.code().refinements.get(&def).copied()
    }

    fn subject(&self, def: DefId) -> Option<DefId> {
        self.code().subjects.get(&def).copied()
    }

    fn state_name(&self, def: DefId) -> String {
        self.code().state_names[&def].clone()
    }

    /// The function an imported name refers to, and the module it lives in.
    fn imported_function(&self, def: DefId) -> Option<(usize, &'a FnDecl)> {
        if self.resolutions().import(def)?.kind != ExportKind::Function {
            return None;
        }
        let module = self.resolutions().import_module(def)?;
        let index = *self.by_path.get(module)?;
        let name = &self.resolutions().def(def).name;

        // Only top level functions are in this map, so a handler operation
        // that happens to share a name cannot be reached by mistake.
        let there = &self.modules[index];
        let function = there
            .functions
            .iter()
            .find(|(id, _)| there.resolutions.def(**id).name == *name)
            .map(|(_, function)| *function)?;
        Some((index, function))
    }

    /// Which module declared a variant, and what it is called there.
    ///
    /// A local variant is declared here. One reached through an import was
    /// declared wherever the import points, and it has to keep that identity
    /// or the same variant would compare unequal to itself depending on which
    /// file the reader came through.
    fn variant_id(&self, def: DefId) -> Option<(Rc<str>, String)> {
        let data = self.resolutions().def(def);
        match data.kind {
            DefKind::Variant => Some((self.here(), data.name.clone())),
            DefKind::Import => {
                if self.resolutions().import(def)?.kind != ExportKind::Variant {
                    return None;
                }
                let module = self.resolutions().import_module(def)?;
                let path = match self.by_path.get(module) {
                    Some(index) => Rc::clone(&self.modules[*index].path),
                    None => Rc::from(module),
                };
                Some((path, data.name.clone()))
            }
            _ => None,
        }
    }

    /// The same, for the property generator, which builds values from outside.
    pub(crate) fn variant_identity(&self, def: DefId) -> Option<(Rc<str>, String)> {
        self.variant_id(def)
    }

    /// Which module declared an effect, and what it is called there.
    fn effect_id(&self, def: DefId) -> Option<Origin> {
        self.effect_id_in(self.current, def)
    }

    /// The same, resolved as the module at `index` sees it.
    fn effect_id_in(&self, index: usize, def: DefId) -> Option<Origin> {
        let there = &self.modules[index];
        let data = there.resolutions.def(def);
        match data.kind {
            DefKind::Effect => Some((Rc::clone(&there.path), data.name.clone())),
            DefKind::Import => {
                if there.resolutions.import(def)?.kind != ExportKind::Effect {
                    return None;
                }
                let module = there.resolutions.import_module(def)?;
                let path = match self.by_path.get(module) {
                    Some(found) => Rc::clone(&self.modules[*found].path),
                    None => Rc::from(module),
                };
                Some((path, data.name.clone()))
            }
            // An operation names its effect through its parent, which is what
            // a `uses` row and a `dispatch` both start from.
            DefKind::EffectOp => {
                let parent = data.parent?;
                self.effect_id_in(index, parent)
            }
            // `Io` is declared by nobody, so it is named by the prelude rather
            // than by whichever module happened to ask. Two modules asking
            // about it have to get the same answer, or a row entry written in
            // one would not be the entry performed in the other.
            DefKind::Builtin => Some((Rc::from(deed_resolve::PRELUDE_MODULE), data.name.clone())),
            _ => None,
        }
    }

    fn def_of(&self, ident: &Ident) -> Option<DefId> {
        self.resolutions().resolution(ident.span)
    }

    /// Calls a function from outside any running program.
    ///
    /// Contract checking happens inside [`Interp::call`], so a caller here gets
    /// precondition and postcondition failures without doing anything, which
    /// is what lets the property runner treat a precondition violation as an
    /// input to discard rather than reimplementing the check.
    pub(crate) fn call_from_outside(
        &mut self,
        function: &'a FnDecl,
        args: Vec<(Value, Span)>,
        span: Span,
    ) -> Result<Value, Box<Diagnostic>> {
        let file = self.file();
        match self.call(function, args, span, file, None) {
            Ok(value) | Err(Signal::Return(value)) => Ok(value),
            Err(Signal::Fail(diagnostic)) => Err(diagnostic),
        }
    }

    /// Whether a value satisfies the refinement `alias` declares.
    pub(crate) fn satisfies(&mut self, alias: DefId, predicate: &'a Expr, value: &Value) -> bool {
        self.eval_predicate(alias, predicate, value)
            .unwrap_or(false)
    }

    pub(crate) fn make(program: &Program<'a>, file: FileId) -> Self {
        Self::new(program, file)
    }

    fn kind_of(&self, def: DefId) -> DefKind {
        self.resolutions().def(def).kind
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("there is always a frame")
    }

    /// Binds a name in the running frame, replacing whatever was there.
    ///
    /// The one caller is `for`, and replacing is the point: each turn binds
    /// the element and the accumulator again rather than assigning to them.
    /// A binding keyed by definition makes that the same operation, and the
    /// definition is out of scope after the loop, so nothing can name what is
    /// left behind.
    fn rebind(&mut self, ident: &Ident, value: Value) {
        if let Some(def) = self.def_of(ident) {
            self.frame().insert(def, value);
        }
    }

    // -- failures ----------------------------------------------------------

    fn fail(&self, diagnostic: Diagnostic) -> Signal {
        Signal::Fail(Box::new(diagnostic))
    }

    /// Something the running program did that `deed check` refuses.
    ///
    /// Nearly everything that arrives here is a shape the type checker turns
    /// down: `+` on an Int and a Bool, a `for` over something that is not a
    /// list, a field on a number, a call with the wrong arity. The note used
    /// to say the opposite, that this was a gap in the interpreter rather than
    /// something the language forbids, so a reader who believed it would go
    /// looking for a missing feature when what they have is an unchecked file
    /// or a hole in the check.
    ///
    /// The "yet" went with it, for the same reason. It promised work that is
    /// not coming: the answer to every one of these is a diagnostic from an
    /// earlier pass, and there is nothing here left to implement.
    ///
    /// The rest of what arrives here are invariants nothing has violated,
    /// argued about at the bottom of `crates/deed-interp/tests/messages.rs`.
    ///
    /// The shapes that are not the checker's business no longer come through
    /// here at all: a call into a module the interpreter was never handed goes
    /// through [`Interp::no_code_for`], and `sys.files` in a program with no
    /// directory through an arm of its own. How this code's messages divide up
    /// is written down once, on [`codes::NOT_RUNNABLE`].
    ///
    /// There used to be a third helper for the two shapes the language allowed
    /// and the interpreter had not implemented, both of them handler state read
    /// from a closure. `DEED4030` refuses those where they are written, so the
    /// language does forbid them now and the note saying otherwise went with
    /// the helper.
    fn not_runnable(&self, span: Span, what: &str) -> Signal {
        self.fail(
            Diagnostic::error(
                codes::NOT_RUNNABLE,
                self.file(),
                span,
                format!("the interpreter cannot run {what}"),
            )
            .with_primary_label("not runnable")
            .with_note(
                "nothing that passes `deed check` reaches this, so either this file was not checked or the check has a hole",
            ),
        )
    }

    /// A call whose body is in a module the interpreter was never given.
    ///
    /// Neither a gap in the interpreter nor a hole in the check: the name
    /// resolved, so the module is known and the call is honest, and what is
    /// missing is code the caller of this library did not hand over. That is
    /// what `crates/deed-interp/src/codes.rs` has said about it all along,
    /// while the message itself carried the note saying the language permits
    /// something the interpreter has not got round to.
    ///
    /// `file` is passed in because one of the two callers has already made the
    /// callee's module current, and `span` is the caller's either way.
    fn no_code_for(&self, file: FileId, span: Span, def: DefId) -> Signal {
        let name = self.resolutions().def(def).name.clone();
        self.fail(
            Diagnostic::error(
                codes::NOT_RUNNABLE,
                file,
                span,
                format!("`{name}` was imported from a module whose code was not handed to the interpreter"),
            )
            .with_primary_label("no body to run")
            .with_note(
                "every module a program calls into has to be in the `Program`, and this one was resolved without being added",
            ),
        )
    }

    // -- expressions -------------------------------------------------------

    /// Evaluates an expression, then makes good on whatever the checker said
    /// about it.
    ///
    /// The guard is here rather than at the handful of places that happened to
    /// need one, because the handful was wrong. A refined argument was checked
    /// and a refined return value was not, so the compiler printed "so it
    /// becomes a runtime check" over a check that did not exist. Hanging it off
    /// the span the checker recorded means the tier says one thing and the two
    /// passes cannot drift apart.
    fn eval(&mut self, expr: &'a Expr) -> Eval<Value> {
        let value = self.eval_inner(expr)?;
        if let Some(guard) = self.code().guards.get(&expr.span()).copied() {
            self.guard(guard, &value, expr.span())?;
        }
        Ok(value)
    }

    fn eval_inner(&mut self, expr: &'a Expr) -> Eval<Value> {
        match expr {
            Expr::Int { value, .. } => Ok(Value::Int(*value)),
            Expr::Str { value, .. } => Ok(Value::Str(value.as_str().into())),
            Expr::Bool { value, .. } => Ok(Value::Bool(*value)),
            Expr::Unit(_) => Ok(Value::Unit),

            Expr::Ident(ident) => self.read(ident),

            Expr::Field { receiver, name, .. } => {
                // A resolved name here is qualification, which resolution
                // already settled, and the only qualified thing that is a value
                // is a variant with no payload.
                if let Some(def) = self.resolutions().resolution(name.span)
                    && let Some((origin, variant)) = self.variant_id(def)
                {
                    return Ok(Value::variant(origin, variant, Fields::new()));
                }

                let receiver_value = self.eval(receiver)?;
                self.field(&receiver_value, receiver.span(), name)
            }

            Expr::Call { callee, args, span } => self.call_expr(callee, args, *span),

            Expr::List { elements, .. } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval(element)?);
                }
                Ok(Value::list(values))
            }

            Expr::StructLit { path, fields, span } => self.build(path, fields, *span),

            Expr::Unary {
                op, operand, span, ..
            } => {
                let value = self.eval(operand)?;
                match (op, value) {
                    (UnaryOp::Neg, Value::Int(n)) => n
                        .checked_neg()
                        .map(Value::Int)
                        .ok_or_else(|| self.overflow(*span)),
                    (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    // Named, like the binary case two arms down. "this
                    // operator on this value" told the reader nothing the
                    // caret had not already told them, and the sibling
                    // handling `1 + true` names both sides.
                    (op, other) => Err(self.not_runnable(
                        *span,
                        &format!("`{}` on {}", unary_op_as_str(*op), other.describe()),
                    )),
                }
            }

            Expr::Binary {
                op, lhs, rhs, span, ..
            } => self.binary(*op, lhs, rhs, *span),

            Expr::Try { operand, span } => {
                let value = self.eval(operand)?;
                match value {
                    Value::Result { ok: true, value } => Ok((*value).clone()),
                    // The rest of the body does not run. That is the whole
                    // point of the operator.
                    failure @ Value::Result { ok: false, .. } => Err(Signal::Return(failure)),
                    other => Err(self.not_runnable(
                        *span,
                        &format!("`?` on {}, which is not a Result", other.describe()),
                    )),
                }
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let taken = self.condition(condition)?;
                if taken {
                    self.eval_block(then_branch)
                } else {
                    match else_branch {
                        Some(else_branch) => self.eval(else_branch),
                        None => Ok(Value::Unit),
                    }
                }
            }

            // A fold, run as one. Nothing is assigned: the binder and the
            // accumulator are bound again on each turn, and the block's value
            // is what the next turn starts with.
            Expr::For {
                binder,
                index,
                iterable,
                accumulator,
                keep,
                body,
                ..
            } => {
                let walked = self.eval(iterable)?;
                let Value::List(elements) = walked else {
                    return Err(self.not_runnable(
                        iterable.span(),
                        &format!("a `for` over {}", walked.describe()),
                    ));
                };

                let mut carried = match accumulator {
                    Some(accumulator) => self.eval(&accumulator.init)?,
                    None => Value::Unit,
                };

                for (position, element) in elements.iter().enumerate() {
                    // Before the turn, and before the element is bound,
                    // because this is deciding whether that turn happens at
                    // all. The accumulator has to be in place first, since
                    // what the walk has worked out so far is the only thing
                    // the condition has to go on.
                    if let Some(keep) = keep {
                        if let Some(accumulator) = accumulator {
                            self.rebind(&accumulator.name, carried.clone());
                        }
                        match self.eval(keep)? {
                            Value::Bool(true) => {}
                            Value::Bool(false) => break,
                            other => {
                                return Err(self.not_runnable(
                                    keep.span(),
                                    &format!("a `for` condition that is {}", other.describe()),
                                ));
                            }
                        }
                    }

                    self.rebind(binder, element.clone());
                    // Zero-based, like `at`. A `for` that counted from one
                    // while the only way to index counted from zero would be a
                    // trap rather than a convenience.
                    if let Some(index) = index {
                        self.rebind(index, Value::Int(position as i64));
                    }
                    if let Some(accumulator) = accumulator {
                        self.rebind(&accumulator.name, carried);
                    }
                    carried = self.eval_block(body)?;
                }

                Ok(carried)
            }

            Expr::Match {
                scrutinee,
                arms,
                span,
            } => {
                let value = self.eval(scrutinee)?;
                for arm in arms {
                    if self.matches(&value, &arm.pattern) {
                        self.bind(&value, &arm.pattern);
                        return self.eval(&arm.body);
                    }
                }
                Err(self.fail(
                    Diagnostic::error(
                        codes::NOT_RUNNABLE,
                        self.file(),
                        *span,
                        format!("no arm of this match accepted {value}"),
                    )
                    .with_primary_label("nothing matched")
                    .with_note("the type checker believes this match is exhaustive, so this is a bug in the interpreter or in the exhaustiveness check"),
                ))
            }

            Expr::Block(block) => self.eval_block(block),

            Expr::Closure {
                params, body, span, ..
            } => {
                let code = match self.closure_at.get(&(self.current, *span)) {
                    Some(found) => *found,
                    None => {
                        self.closures.push(Closure {
                            module: self.current,
                            params,
                            body,
                        });
                        let index = self.closures.len() - 1;
                        self.closure_at.insert((self.current, *span), index);
                        index
                    }
                };
                // Everything visible right now, by value. A closure cannot
                // leave the function that wrote it, so copying the frame is
                // never strictly necessary today; doing it anyway means the
                // answer does not depend on that staying true.
                Ok(Value::Closure(Rc::new(ClosureValue {
                    code,
                    captured: self.frames.last().cloned().unwrap_or_default(),
                })))
            }

            Expr::Old { span, .. } => match self.olds.last().and_then(|olds| olds.get(span)) {
                Some(value) => Ok(value.clone()),
                None => Err(self.not_runnable(*span, "`old` outside a contract")),
            },

            Expr::Unchanged { effect, span } => {
                // The same rule `old` has had all along, said out loud. Both
                // of these read what entering a call captured, and a call
                // captures for the sake of its `ensures` clauses, so outside
                // one there is nothing for either of them to be about. `old`
                // refused here already, by looking up a span no captured map
                // contains; this one used to answer from whatever the nearest
                // call that captured had, which is a different call.
                if self.in_contract == 0 {
                    return Err(self.not_runnable(*span, "`unchanged` outside a contract"));
                }
                let Some(def) = self.def_of(&effect.effect) else {
                    return Err(self.not_runnable(*span, "this effect reference"));
                };
                let Some(id) = self.effect_id(def) else {
                    return Err(self.not_runnable(*span, "this effect reference"));
                };
                let before = self
                    .entry_states
                    .last()
                    .and_then(|states| states.get(&id))
                    .cloned();
                let now = self.state_of(&id);
                Ok(Value::Bool(before == now))
            }

            Expr::With {
                handlers,
                body,
                finally,
                ..
            } => {
                let base = self.handlers.len();
                let mut installed = Ok(());
                for handler in handlers {
                    installed = self.install(handler);
                    if installed.is_err() {
                        break;
                    }
                }
                let body_result = match installed {
                    Ok(()) => self.eval_block(body),
                    Err(signal) => Err(signal),
                };
                // Run `finally` blocks for every installed handler that has
                // one, from the most recently installed to the least recently
                // installed. This runs whether the body returned normally,
                // via `return`, or via a contract failure so that any
                // resource a handler acquired is always released.
                let result = self.run_finally_blocks(base, result);
                self.handlers.truncate(base);

                // Run the `finally` clause on every exit: normal completion,
                // contract failure, and abandonment all pass through here.
                // If the `finally` clause itself raises a signal that takes
                // priority over what the body raised.
                match finally {
                    Some(finally_block) => match self.eval_block(finally_block) {
                        Ok(_) | Err(Signal::Return(_)) => body_result,
                        Err(signal) => Err(signal),
                    },
                    None => body_result,
                }
            }

            Expr::Error(span) => Err(self.not_runnable(*span, "code that did not compile")),
        }
    }

    fn condition(&mut self, expr: &'a Expr) -> Eval<bool> {
        let value = self.eval(expr)?;
        value
            .as_bool()
            .ok_or_else(|| self.not_runnable(expr.span(), "a condition that is not a Bool"))
    }

    /// Runs something that is part of a contract rather than part of the
    /// program.
    ///
    /// A `where` or `ensures` clause reads state to describe it, and a
    /// contract does not contribute to a row, so a read that happens here is
    /// not something the signature has to admit to. Marked rather than
    /// inferred: the alternative is working out afterwards which effects were
    /// the contract's, which is the same question with less to go on.
    fn in_a_contract<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        self.in_contract += 1;
        let started = self.profile.as_ref().map(|_| Instant::now());
        let result = run(self);
        if let Some(started) = started {
            self.add_contract_time(started.elapsed());
        }
        self.in_contract -= 1;
        result
    }

    /// What a definition is bound to in the running frame, if anything.
    fn lookup(&self, def: DefId) -> Option<Value> {
        self.frames.last()?.get(&def).cloned()
    }

    /// What a name holds, wherever the value it holds lives.
    ///
    /// A name is looked up in the frame everywhere except one: handler state
    /// lives in the handler instance instead. Everything that reads a name
    /// goes through [`Interp::read`], which knows that, but calling one goes
    /// through the callee expression, which used to ask the frame alone. So a
    /// closure kept in handler state could be stored and never called: the
    /// value was there and `held()` said the interpreter could not run the
    /// call.
    fn bound_value(&self, def: DefId) -> Option<Value> {
        if self.kind_of(def) == DefKind::State {
            let index = self.inside_handler.last().copied()?;
            return self.handlers[index]
                .state
                .get(&self.state_name(def))
                .cloned();
        }
        self.lookup(def)
    }

    fn read(&mut self, ident: &Ident) -> Eval<Value> {
        let Some(def) = self.def_of(ident) else {
            return Err(self.not_runnable(ident.span, "an unresolved name"));
        };

        if self.kind_of(def) == DefKind::State {
            return self.read_state(def, ident.span);
        }
        // A variant with no payload is a value. One reached through an import
        // is the same value as the one built where it was declared, which is
        // what `variant_id` is for.
        if let Some((origin, variant)) = self.variant_id(def) {
            return Ok(Value::variant(origin, variant, Fields::new()));
        }

        match self.frames.last().and_then(|frame| frame.get(&def)) {
            Some(value) => Ok(value.clone()),
            // A declared function named where a value belongs. It is not in
            // any frame, because it is not a binding, and it is a value all
            // the same: `apply(double, 3)` has to hand `double` over somehow.
            None if matches!(self.kind_of(def), DefKind::Function | DefKind::Import) => {
                Ok(Value::Function {
                    module: self.current,
                    def,
                })
            }
            None => Err(self.not_runnable(
                ident.span,
                &format!("`{}`, which has no value here", ident.name),
            )),
        }
    }

    /// Reads handler state, out of the handler whose operation is running.
    ///
    /// Both refusals here are the same gap rather than two, and neither is
    /// reachable from a file `deed check` accepts. A state name is only in
    /// scope inside the handler that declared it, and the one shape that could
    /// carry a read out of the running operation was a closure: a closure
    /// captures the frame and the frame is not where state lives, so calling
    /// it after the operation returned looked in no handler at all and calling
    /// it inside another handler's operation looked in that one's table. When
    /// the two handlers shared a state name there was no refusal and no
    /// message, and the closure quietly answered with the other handler's
    /// number. `DEED4030` refuses that where it is written instead.
    fn read_state(&self, def: DefId, span: Span) -> Eval<Value> {
        let Some(index) = self.inside_handler.last().copied() else {
            return Err(self.not_runnable(span, "handler state from outside a handler"));
        };
        let name = self.state_name(def);
        match self.handlers[index].state.get(&name) {
            Some(value) => Ok(value.clone()),
            None => Err(self.not_runnable(span, "handler state that was never initialised")),
        }
    }

    /// The state of the handler currently installed for `effect`.
    fn state_of(&self, effect: &Origin) -> Option<Fields> {
        self.handlers
            .iter()
            .rev()
            .find(|instance| instance.effect == *effect)
            .map(|instance| instance.state.clone())
    }

    fn field(&self, value: &Value, receiver: Span, name: &Ident) -> Eval<Value> {
        // `System` carries narrower capabilities. Taking one out is how
        // authority gets delegated, and it only ever narrows.
        if let Value::Capability(Capability::System) = value {
            return match name.name.as_str() {
                "console" => Ok(Value::Capability(Capability::Console)),
                "clock" => Ok(Value::Capability(Capability::Clock)),
                "files" => match &self.root {
                    Some(root) => Ok(Value::Capability(Capability::Dir(Rc::clone(root)))),
                    // Nothing granted a directory, so there is not one to hand
                    // out. Inventing the working directory here would be the
                    // ambient authority the whole design is against.
                    //
                    // The one message in this crate that a reader meets on a
                    // file that checked and a program that is right. It used
                    // to be phrased as a gap in the interpreter, which sent
                    // whoever typed `deed run --dir` at a directory that is
                    // not there looking in the compiler for their own typo.
                    None => Err(self.fail(
                        Diagnostic::error(
                            codes::NOT_RUNNABLE,
                            self.file(),
                            name.span,
                            "this program was not given a directory",
                        )
                        .with_primary_label("there is no `Dir` to hand out")
                        .with_note(
                            "`sys.files` hands out the directory the run was rooted at; `deed run` roots it at `--dir`, or at the working directory when there is no `--dir`, and a path it cannot open leaves the program with none",
                        ),
                    )),
                },
                other => Err(self.not_runnable(
                    name.span,
                    &format!("`System.{other}`, which does not exist"),
                )),
            };
        }

        let fields = match value {
            Value::Record(fields) => &**fields,
            Value::Variant(variant) => &variant.fields,
            // The receiver rather than the name. Nothing is wrong with the
            // name: a value of the right shape would have had it, and what
            // the reader has to look at is the thing that turned out not to
            // be that shape.
            other => {
                return Err(
                    self.not_runnable(receiver, &format!("field access on {}", other.describe()))
                );
            }
        };

        match fields.get(&name.name) {
            Some(value) => Ok(value.clone()),
            None => Err(self.not_runnable(
                name.span,
                &format!("`{}`, which the value does not have", name.name),
            )),
        }
    }

    fn binary(&mut self, op: BinaryOp, lhs: &'a Expr, rhs: &'a Expr, span: Span) -> Eval<Value> {
        use BinaryOp::*;

        // Short circuit before touching the right hand side, since the right
        // hand side can perform effects.
        if matches!(op, And | Or) {
            let left = self.condition(lhs)?;
            return match (op, left) {
                (And, false) => Ok(Value::Bool(false)),
                (Or, true) => Ok(Value::Bool(true)),
                _ => Ok(Value::Bool(self.condition(rhs)?)),
            };
        }

        let left = self.eval(lhs)?;
        let right = self.eval(rhs)?;

        if matches!(op, Eq | Ne) {
            let equal = left == right;
            return Ok(Value::Bool(if op == Eq { equal } else { !equal }));
        }

        // Strings join and compare. Ordering is by bytes, which for text that
        // is all one script is the order anybody expects, and for text that is
        // not is a decision nobody should be making without a locale.
        if let (Value::Str(a), Value::Str(b)) = (&left, &right) {
            return match op {
                Add => Ok(Value::str(format!("{a}{b}"))),
                Lt => Ok(Value::Bool(a < b)),
                Le => Ok(Value::Bool(a <= b)),
                Gt => Ok(Value::Bool(a > b)),
                Ge => Ok(Value::Bool(a >= b)),
                _ => Err(self.not_runnable(span, &format!("`{}` on two Strings", op.as_str()))),
            };
        }

        let (Some(a), Some(b)) = (left.as_int(), right.as_int()) else {
            return Err(self.not_runnable(
                span,
                &format!(
                    "`{}` on {} and {}",
                    op.as_str(),
                    left.describe(),
                    right.describe()
                ),
            ));
        };

        let value = match op {
            Add => a.checked_add(b).map(Value::Int),
            Sub => a.checked_sub(b).map(Value::Int),
            Mul => a.checked_mul(b).map(Value::Int),
            Div => a.checked_div(b).map(Value::Int),
            Rem => a.checked_rem(b).map(Value::Int),
            Lt => Some(Value::Bool(a < b)),
            Le => Some(Value::Bool(a <= b)),
            Gt => Some(Value::Bool(a > b)),
            Ge => Some(Value::Bool(a >= b)),
            Eq | Ne | And | Or => unreachable!("handled above"),
        };

        value.ok_or_else(|| self.overflow(span))
    }

    fn overflow(&self, span: Span) -> Signal {
        self.fail(
            Diagnostic::error(
                codes::ARITHMETIC,
                self.file(),
                span,
                "this arithmetic has no answer",
            )
            .with_primary_label("overflow, or division by zero")
            .with_note("`Int` is a 64 bit signed integer and does not wrap"),
        )
    }

    // -- calls -------------------------------------------------------------

    fn call_expr(&mut self, callee: &'a Expr, args: &'a [Expr], span: Span) -> Eval<Value> {
        let def = match callee {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions().resolution(name.span),
            _ => None,
        };

        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push((self.eval(arg)?, arg.span()));
        }

        // A closure is a value, not a declaration, so it is reached by looking
        // at what the callee evaluates to rather than at what it resolves to.
        if let Expr::Ident(ident) = callee
            && let Some(def) = self.def_of(ident)
            && let Some(bound) = self.bound_value(def)
        {
            match bound {
                Value::Closure(closure) => return self.call_closure(&closure, values, span),
                // A function handed over as a value. Called through the same
                // path a written-out call takes, so its contract is checked
                // rather than skipped by having been passed rather than named.
                Value::Function { module, def } => {
                    return self.call_declared(module, def, values, span, callee.span());
                }
                _ => {}
            }
        }

        let Some(def) = def else {
            return Err(self.not_runnable(callee.span(), "this call"));
        };

        match self.kind_of(def) {
            DefKind::Function => {
                let Some(function) = self.function(def) else {
                    return Err(self.not_runnable(callee.span(), "this call"));
                };
                let here = self.file();
                self.call(function, values, span, here, None)
            }
            DefKind::EffectOp => self.dispatch(def, values, span),
            DefKind::Builtin => {
                let name = self.resolutions().def(def).name.clone();
                let carried = values.first().map(|(value, _)| value.clone());
                match (name.as_str(), carried) {
                    ("ok", Some(value)) => Ok(Value::ok(value)),
                    ("err", Some(value)) => Ok(Value::err(value)),
                    // Characters, not bytes. A length that counted bytes would
                    // make `length("é")` two, and a refinement written against
                    // it would mean something different depending on which
                    // letters happened to be in the string.
                    ("length", Some(Value::Str(text))) => {
                        Ok(Value::Int(text.chars().count() as i64))
                    }
                    ("length", Some(Value::List(elements))) => {
                        Ok(Value::Int(elements.len() as i64))
                    }
                    // An index nobody promised is there. Nothing in this
                    // language stops a program, so it comes back as an error
                    // value and the caller has to say what to do about it.
                    ("at", Some(Value::List(elements))) => {
                        let index = values.get(1).and_then(|(value, _)| value.as_int());
                        let found = index
                            .filter(|index| *index >= 0)
                            .and_then(|index| elements.get(index as usize));
                        Ok(match found {
                            Some(value) => Value::ok(value.clone()),
                            None => Value::err(Value::str(format!(
                                "index {} is outside a list of {}",
                                index.unwrap_or(0),
                                elements.len()
                            ))),
                        })
                    }
                    ("push", Some(Value::List(elements))) => {
                        let Some((value, _)) = values.get(1) else {
                            return Err(self.not_runnable(callee.span(), "this call"));
                        };
                        let mut extended = (*elements).clone();
                        extended.push(value.clone());
                        Ok(Value::list(extended))
                    }
                    // Having something a number of times, which a `for` cannot
                    // give because a `for` needs a list before it starts.
                    //
                    // A count of zero or less is the empty list rather than a
                    // refusal. The call this exists for is
                    // `repeat(" ", width - length(text))`, which goes negative
                    // exactly when the text is already wider than the column,
                    // and what it means there is no padding.
                    ("repeat", Some(value)) => {
                        let Some(count) = values.get(1).and_then(|(value, _)| value.as_int())
                        else {
                            return Err(self.not_runnable(callee.span(), "this call"));
                        };
                        let count = usize::try_from(count).unwrap_or(0);
                        Ok(Value::list(vec![value; count]))
                    }
                    ("split", Some(Value::Str(text))) => {
                        let Some((Value::Str(separator), _)) = values.get(1) else {
                            return Err(self.not_runnable(callee.span(), "this call"));
                        };
                        Ok(Value::list(split(&text, separator)))
                    }
                    ("join", Some(Value::List(elements))) => {
                        let Some((Value::Str(separator), _)) = values.get(1) else {
                            return Err(self.not_runnable(callee.span(), "this call"));
                        };
                        let mut text = String::new();
                        for (index, element) in elements.iter().enumerate() {
                            let Value::Str(piece) = element else {
                                return Err(self.not_runnable(callee.span(), "this call"));
                            };
                            if index > 0 {
                                text.push_str(separator);
                            }
                            text.push_str(piece);
                        }
                        Ok(Value::str(text))
                    }
                    ("to_string", Some(Value::Int(number))) => Ok(Value::str(number.to_string())),
                    // Space, tab, carriage return and newline, and not the
                    // Unicode whitespace table. That is a large amount of
                    // behaviour to hide behind a four letter name, and it
                    // would make what this does depend on a table nobody
                    // reading the signature can see.
                    ("trim", Some(Value::Str(text))) => {
                        Ok(Value::str(text.trim_matches(WHITESPACE)))
                    }
                    // The twenty-six letters and nothing else, for the reason
                    // `trim` gives above: deciding what an uppercase letter is
                    // in general needs a table, and a name this short may not
                    // depend on one a reader cannot see. Every other character
                    // comes back as it went in, so text in a script with no
                    // case survives rather than being mangled by a rule that
                    // was not written for it.
                    ("upper", Some(Value::Str(text))) => Ok(Value::str(text.to_ascii_uppercase())),
                    ("lower", Some(Value::Str(text))) => Ok(Value::str(text.to_ascii_lowercase())),
                    // Text that is not a number is not a mistake in the
                    // caller: it usually came from a file or an argument, and
                    // deciding what to do about it is the caller's job.
                    ("to_int", Some(Value::Str(text))) => Ok(match text.parse::<i64>() {
                        Ok(number) => Value::ok(Value::Int(number)),
                        Err(_) => Value::err(Value::str(format!("`{text}` is not a number"))),
                    }),
                    _ => Err(self.not_runnable(callee.span(), "this call")),
                }
            }
            // The body lives in another module and so do its names, so the
            // call runs with that module current. Reading the callee's names
            // out of the caller's scope would be a class of bug that does not
            // announce itself.
            DefKind::Import => {
                let here = self.file();
                match self.imported_function(def) {
                    Some((module, function)) => {
                        let caller = self.current;
                        self.current = module;
                        let result = self.call(function, values, span, here, None);
                        self.current = caller;
                        result
                    }
                    None => Err(self.no_code_for(here, callee.span(), def)),
                }
            }
            _ => Err(self.not_runnable(callee.span(), "this call")),
        }
    }

    /// Calls a function that was handed over as a value.
    ///
    /// The module is part of what was handed over, because a definition is an
    /// index into one module's table and reading the callee's names out of the
    /// caller's scope is a class of bug that does not announce itself.
    ///
    /// Nothing reaches the refusals below, which want a function value whose
    /// definition is an import with no body behind it, so swapping the file
    /// they are filed against leaves every test green. The caller's file is
    /// still threaded through rather than read off `self`, because it is the
    /// same rule as the one [`Interp::call_body`] is held to, and a site that
    /// only has to be right sometimes is a site that stops being right.
    fn call_declared(
        &mut self,
        module: usize,
        def: DefId,
        args: Vec<(Value, Span)>,
        span: Span,
        callee_span: Span,
    ) -> Eval<Value> {
        let caller = self.current;
        let here = self.file();
        self.current = module;

        // Every span in the failures below is the caller's, so the caller's
        // module goes back on the front before one is built. `self.file()` is
        // whatever is current, and a diagnostic filed against the callee with
        // the caller's byte offsets underlines a line in the wrong file.
        let result = match self.kind_of(def) {
            DefKind::Function => match self.function(def) {
                Some(function) => self.call(function, args, span, here, None),
                None => {
                    self.current = caller;
                    Err(self.not_runnable(callee_span, "this call"))
                }
            },
            DefKind::Import => match self.imported_function(def) {
                Some((module, function)) => {
                    self.current = module;
                    self.call(function, args, span, here, None)
                }
                None => Err(self.no_code_for(here, callee_span, def)),
            },
            _ => {
                self.current = caller;
                Err(self.not_runnable(callee_span, "this call"))
            }
        };

        self.current = caller;
        result
    }

    fn dispatch(&mut self, operation: DefId, args: Vec<(Value, Span)>, span: Span) -> Eval<Value> {
        let name = self.resolutions().def(operation).name.clone();
        let Some(effect) = self.resolutions().def(operation).parent else {
            return Err(self.not_runnable(span, "this effect operation"));
        };

        // The built-in effect has no handler to install, because the handler is
        // the outside world. Which part of it is decided by the capability that
        // was passed in, not by anything in scope.
        if self.resolutions().builtin("Io") == Some(effect) {
            self.hold_to_row(effect, &name, None, span)?;
            return self.perform_io(&name, &args, span);
        }

        let effect_name = self.resolutions().def(effect).name.clone();
        let id = self.effect_id(effect);
        let Some(index) = id
            .as_ref()
            .and_then(|id| self.handlers.iter().rposition(|found| found.effect == *id))
        else {
            return Err(self.fail(
                Diagnostic::error(
                    codes::NO_HANDLER,
                    self.file(),
                    span,
                    format!("no handler is installed for `{effect_name}`"),
                )
                .with_primary_label("nothing can perform this")
                .with_note("wrap the call in a `with` block naming a handler for the effect"),
            ));
        };

        let handler_def = self.handlers[index].handler;
        let home = self.handlers[index].module;
        self.hold_to_row(effect, &name, Some(index), span)?;
        let Some(declaration) = self.modules[home].handler_decls.get(&handler_def).copied() else {
            return Err(self.not_runnable(span, "this handler"));
        };
        let Some(operation_decl) = declaration
            .operations
            .iter()
            .find(|candidate| candidate.sig.name.name == name)
        else {
            // Unreachable from a file that checked, since DEED4029 refuses a
            // handler that leaves an operation out. Kept, and pointed at the
            // compiler rather than the program for the same reason DEED6010
            // is: the file was accepted, so a gap reaching here means the
            // check that accepted it was wrong.
            let handler_name = declaration.name.name.clone();
            return Err(self.fail(
                Diagnostic::error(
                    codes::NO_HANDLER,
                    self.file(),
                    span,
                    format!("the handler `{handler_name}` does not implement `{name}`"),
                )
                .with_primary_label("not implemented")
                .with_secondary(declaration.name.span, "this handler")
                .with_note(
                    "a handler has to implement every operation its effect declares, and this file was accepted, so this is a hole in the type checker rather than a mistake in the program; please report it",
                ),
            ));
        };

        let caller = self.current;
        let here = self.file();
        self.current = home;
        let result = self.call(operation_decl, args, span, here, Some(index));
        self.current = caller;
        result
    }

    /// Performs a built-in operation.
    ///
    /// Every one of these takes the capability it acts on as its first
    /// argument, so a function that was not handed one cannot reach the
    /// outside world. Two of them, `open` and `make`, hand one back, and the
    /// rule that keeps the mechanism honest is not that a capability cannot be
    /// produced but that what comes back reaches strictly less than what went
    /// in. Nothing widens. `crates/deed-driver/tests/capabilities.rs` tests
    /// that for both, and `deed_typeck::io_signatures` is where the set of two
    /// is counted.
    fn perform_io(&mut self, name: &str, args: &[(Value, Span)], span: Span) -> Eval<Value> {
        let capability = match args.first() {
            Some((Value::Capability(capability), _)) => capability.clone(),
            _ => {
                return Err(self.not_runnable(span, "an `Io` operation with no capability"));
            }
        };

        match (name, capability) {
            ("write", Capability::Console) => {
                let line = match args.get(1) {
                    Some((Value::Str(text), _)) => text.to_string(),
                    Some((other, _)) => other.to_string(),
                    None => String::new(),
                };
                self.output.push(line);
                Ok(Value::Unit)
            }
            // A monotonic count of milliseconds. Wall clock time would make
            // every run different, and P8 says the default is deterministic.
            ("now", Capability::Clock) => {
                self.ticks += 1;
                Ok(Value::Int(self.ticks))
            }
            // The machine's clock, in milliseconds since 1970. Separate from
            // `now` for the same reason `save` is separate from `read`: the
            // capability says which resource, the row says what is being done
            // to it, and the difference here is whether the program can give
            // the same answer twice.
            //
            // Negative before 1970 rather than an error. A clock set that far
            // back is a machine that is wrong, and the honest number for it is
            // a negative one; refusing to answer would be the operation
            // deciding it knows better than the machine it was asked about.
            ("epoch", Capability::Clock) => {
                let now = std::time::SystemTime::now();
                let millis = match now.duration_since(std::time::UNIX_EPOCH) {
                    Ok(since) => since.as_millis() as i64,
                    Err(before) => -(before.duration().as_millis() as i64),
                };
                Ok(Value::Int(millis))
            }
            // Data rather than authority, which is why it hands back a list
            // instead of something opaque. It takes the root capability all
            // the same, so a function that wants the arguments has to have
            // been handed everything, and everything below `main` gets the
            // values passed down like any other input.
            ("args", Capability::System) => Ok(Value::list(
                self.arguments
                    .iter()
                    .map(|argument| Value::Str(Rc::clone(argument)))
                    .collect(),
            )),
            // Enumerating rather than naming. A `Dir` plus `read` lets a
            // program read the file somebody told it about; a `Dir` plus
            // `list` lets it find out what is there, which is strictly more
            // and is why it is a separate entry in the row rather than
            // something `read` happens to allow.
            //
            // Files only, and sorted. Sorted because a caller that depends on
            // the order the filesystem felt like today is a caller with a bug
            // that appears on somebody else's machine. Files only because a
            // list holding two kinds of thing with no way to tell them apart
            // is the sort of thing that turns into a bug in the caller, and
            // nothing has yet needed to discover a subdirectory.
            ("list", Capability::Dir(root)) => Ok(match std::fs::read_dir(&*root) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|entry| {
                            let entry = entry.ok()?;
                            entry.file_type().ok()?.is_file().then_some(())?;
                            entry.file_name().into_string().ok()
                        })
                        .collect();
                    names.sort();
                    Value::ok(Value::list(names.into_iter().map(Value::str).collect()))
                }
                Err(error) => Value::err(Value::str(format!("{error}"))),
            }),
            // Narrowing. The `Dir` that comes back reaches strictly less than
            // the one that went in, and there is no operation that goes the
            // other way, so authority only ever shrinks on the way down.
            ("open", Capability::Dir(root)) => {
                let name_arg = self.io_name(args.get(1), span)?;
                Ok(match sandbox::resolve(&root, &name_arg) {
                    Ok(path) if path.is_dir() => {
                        Value::ok(Value::Capability(Capability::Dir(Rc::from(path))))
                    }
                    Ok(_) => Value::err(Value::str(format!("`{name_arg}` is not a directory"))),
                    Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                })
            }
            ("read", Capability::Dir(root)) => {
                let name_arg = self.io_name(args.get(1), span)?;
                Ok(match sandbox::resolve(&root, &name_arg) {
                    Ok(path) => match std::fs::read_to_string(&path) {
                        Ok(text) => Value::ok(Value::str(text)),
                        Err(error) => Value::err(Value::str(format!("`{name_arg}`: {error}"))),
                    },
                    Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                })
            }
            // Writing goes through the same rules about names as reading, and
            // through a resolver that allows a file which is not there yet
            // without allowing one that is somewhere else.
            ("save", Capability::Dir(root)) => {
                let name_arg = self.io_name(args.get(1), span)?;
                let contents = match args.get(2) {
                    Some((Value::Str(text), _)) => text.to_string(),
                    Some((other, at)) => {
                        return Err(self.not_runnable(
                            *at,
                            &format!("a file's contents that are {}", other.describe()),
                        ));
                    }
                    None => return Err(self.not_runnable(span, "a save with nothing to save")),
                };
                Ok(match sandbox::resolve_new(&root, &name_arg) {
                    Ok(path) => match std::fs::write(&path, contents) {
                        Ok(()) => Value::ok(Value::Unit),
                        Err(error) => Value::err(Value::str(format!("`{name_arg}`: {error}"))),
                    },
                    Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                })
            }
            // Destroying rather than replacing, which is why it is its own
            // entry in the row and not something `save` happens to allow. A
            // program that writes the wrong bytes can be put back from what it
            // overwrote; one that deletes the wrong file cannot be put back
            // from anything.
            //
            // Files only, like `list`. Removing a directory is a different
            // operation with a different blast radius and nothing here wants
            // it. A name that is not there is an error rather than a success,
            // because "it was already gone" and "I removed it" are different
            // answers.
            //
            // `resolve` rather than `resolve_new`: the file has to be there,
            // and a symlink pointing out of the directory is refused rather
            // than followed out of it.
            ("remove", Capability::Dir(root)) => {
                let name_arg = self.io_name(args.get(1), span)?;
                Ok(match sandbox::resolve(&root, &name_arg) {
                    Ok(path) if path.is_dir() => {
                        Value::err(Value::str(format!("`{name_arg}` is a directory")))
                    }
                    Ok(path) => match std::fs::remove_file(&path) {
                        Ok(()) => Value::ok(Value::Unit),
                        Err(error) => Value::err(Value::str(format!("`{name_arg}`: {error}"))),
                    },
                    Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                })
            }
            // Making a place rather than putting something in one, which is
            // why the answer is a `Dir` and not a `()`. The one it hands back
            // is rooted inside the one it was given, so authority still only
            // shrinks: this is `open` on a directory that did not exist yet.
            //
            // Nothing may already be at the name, file or directory. "I made
            // it" and "it was already there" are different answers, and a
            // program that cannot tell them apart has a bug waiting, which is
            // the same reason a missing file is an error for `remove`.
            //
            // `resolve_new` because the name is not supposed to exist, and it
            // is the resolver that refuses a symlink sitting there pointing
            // out of the directory rather than following it.
            ("make", Capability::Dir(root)) => {
                let name_arg = self.io_name(args.get(1), span)?;
                Ok(match sandbox::resolve_new(&root, &name_arg) {
                    Ok(path) if path.exists() => {
                        Value::err(Value::str(format!("`{name_arg}` is already there")))
                    }
                    Ok(path) => match std::fs::create_dir(&path) {
                        Ok(()) => match sandbox::root(&path) {
                            Ok(made) => {
                                Value::ok(Value::Capability(Capability::Dir(Rc::from(made))))
                            }
                            Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                        },
                        Err(error) => Value::err(Value::str(format!("`{name_arg}`: {error}"))),
                    },
                    Err(refused) => Value::err(Value::str(refused.message(&name_arg))),
                })
            }
            (_, held) => Err(self.fail(
                Diagnostic::error(
                    codes::NO_HANDLER,
                    self.file(),
                    span,
                    format!("`Io.{name}` cannot be performed with a `{}`", held.name()),
                )
                .with_primary_label("wrong capability")
                .with_note("each operation acts on the kind of capability it was given"),
            )),
        }
    }

    /// The name argument of a filesystem operation.
    fn io_name(&self, arg: Option<&(Value, Span)>, span: Span) -> Eval<String> {
        match arg {
            Some((Value::Str(text), _)) => Ok(text.to_string()),
            Some((other, at)) => Err(self.not_runnable(
                *at,
                &format!("a filesystem name that is {}", other.describe()),
            )),
            None => Err(self.not_runnable(span, "a filesystem operation with no name")),
        }
    }

    /// Calls a closure, in the frame it was written in.
    ///
    /// No contract, because a closure cannot carry one, and no effect
    /// discharge, because the effects were already charged to the function
    /// that wrote it. That is a conservative rule rather than the right one:
    /// the right one puts the row in the closure's type and charges the call
    /// site, and it is still an open question in design/03-effects.md.
    fn call_closure(
        &mut self,
        closure: &ClosureValue,
        args: Vec<(Value, Span)>,
        span: Span,
    ) -> Eval<Value> {
        // Before the module switch below. `span` is the call, which was
        // written here, and once `self.current` is the closure's module
        // `self.file()` is a different file for the same bytes.
        //
        // Unlike the same fix on [`Interp::call`], nothing can currently see
        // the difference, and it is worth saying why rather than leaving the
        // next reader to wonder. A closure cannot name itself, so a recursion
        // through one has to fetch it again at every turn, and that fetch is a
        // call one frame deeper than the closure call it feeds. The depth
        // limit therefore always trips on the fetch first. The two paths say
        // the same thing anyway, because the one that says something else is
        // the one that will be wrong when a closure can be reached without a
        // call in front of it.
        let call_file = self.file();

        let Some(&Closure {
            module,
            params,
            body,
        }) = self.closures.get(closure.code)
        else {
            return Err(self.not_runnable(span, "a closure the interpreter lost track of"));
        };

        if args.len() != params.len() {
            return Err(self.not_runnable(span, "a closure called with the wrong arity"));
        }

        let mut frame = closure.captured.clone();
        for (param, (value, _)) in params.iter().zip(&args) {
            if let Some(def) = self.modules[module].resolutions.resolution(param.name.span) {
                frame.insert(def, value.clone());
            }
        }

        let caller = self.current;
        self.current = module;
        self.frames.push(frame);
        let result = self
            .too_deep(call_file, span)
            .and_then(|()| self.eval(body));
        self.frames.pop();
        self.current = caller;
        result
    }

    fn call(
        &mut self,
        function: &'a FnDecl,
        args: Vec<(Value, Span)>,
        call_span: Span,
        call_file: FileId,
        handler: Option<usize>,
    ) -> Eval<Value> {
        let profile_call = self.enter_profiled_call(function);
        let plan = self.plan_of(function);

        let mut frame = Frame::default();
        for (def, (value, _)) in plan.params.iter().zip(&args) {
            if let Some(def) = def {
                frame.insert(*def, value.clone());
            }
        }

        self.frames.push(frame);
        self.rows.push(RowFrame {
            handled: self.handlers.len(),
            handler,
            promise: plan.promise.clone(),
        });
        if let Some(handler) = handler {
            self.inside_handler.push(handler);
        }

        let result = self
            .too_deep(call_file, call_span)
            .and_then(|()| self.call_body(function, call_span, call_file, plan.captures));

        if handler.is_some() {
            self.inside_handler.pop();
        }
        self.rows.pop();
        self.frames.pop();
        self.finish_profiled_call(profile_call, handler.is_some());
        result
    }

    /// What a call to `function` needs, worked out once and kept.
    fn plan_of(&mut self, function: &'a FnDecl) -> Rc<CallPlan> {
        let span = function.sig.name.span;
        if let Some(plan) = self.code().plans.get(&span) {
            return Rc::clone(plan);
        }

        let resolutions = self.resolutions();
        let plan = Rc::new(CallPlan {
            params: function
                .sig
                .params
                .iter()
                .map(|param| resolutions.resolution(param.name.span))
                .collect(),
            promise: self.promise_of(function).map(Rc::new),
            captures: !function.contract.ensures.is_empty(),
        });

        let current = self.current;
        self.modules[current].plans.insert(span, Rc::clone(&plan));
        plan
    }

    /// The row a declaration wrote down, when it says something a call can be
    /// held to.
    fn promise_of(&self, function: &'a FnDecl) -> Option<Promise> {
        let items = self.code().rows.get(&function.sig.name.span)?;
        // A row variable stands for whatever the caller passed, so the
        // declaration alone does not say what this call may perform and there
        // is nothing here to hold it to. The caller's own frame is where that
        // question has an answer, and it is still checked.
        if items
            .iter()
            .any(|item| self.resolutions().def(item.effect).kind == DefKind::RowParam)
        {
            return None;
        }
        let allowed = items
            .iter()
            .filter_map(|item| Some((self.effect_id(item.effect)?, item.operation.clone())))
            .collect();
        Some(Promise {
            name: function.sig.name.name.clone(),
            allowed,
        })
    }

    /// Holds every active call to the row it declared.
    ///
    /// The rows are the argument this language is making, and the pass that
    /// produces them used to be the only thing that ever read one. This is the
    /// program itself disagreeing: an effect just happened, and here is a
    /// function on the stack that said it would not do that.
    ///
    /// Walked innermost first, because the two things that end the walk are
    /// both about how far out the effect reaches. A `with` block inside a frame
    /// answers for what is under it, so that frame and everything outside it
    /// are done with. And a handler operation charges what it performs to
    /// whoever installed the handler, so the frames between the `with` and here
    /// are not asked, which is what `barrier` is for.
    ///
    /// `answered_by` is the handler that took the operation, or `None` for the
    /// built-in effect, which nothing installs and every frame therefore owes.
    fn check_row(
        &self,
        effect: &Origin,
        operation: &str,
        answered_by: Option<usize>,
    ) -> Option<String> {
        let mut barrier = usize::MAX;
        for frame in self.rows.iter().rev() {
            // The `with` that answered this is inside the frame, so the frame
            // discharged it, and so did everything further out.
            if let Some(index) = answered_by
                && frame.handled <= index
            {
                return None;
            }

            if frame.handled <= barrier
                && let Some(promise) = &frame.promise
            {
                let covered = promise.allowed.iter().any(|(declared, op)| {
                    declared == effect && op.as_deref().is_none_or(|name| name == operation)
                });
                if !covered {
                    return Some(promise.name.clone());
                }
            }

            // A handler operation. What it performs belongs to the `with` that
            // installed it, so nothing between here and there is asked.
            if let Some(index) = frame.handler {
                barrier = barrier.min(index);
            }
        }
        None
    }

    /// Reports the first active call that did not declare what just happened.
    ///
    /// An error rather than a warning, and pointed at the compiler rather than
    /// at the program: the checker accepted this file, so if an effect got
    /// through then the check was wrong. Five ways for one to get through were
    /// found by hand and fixed in #131. This is how the next one announces
    /// itself.
    fn hold_to_row(
        &mut self,
        effect: DefId,
        operation: &str,
        answered_by: Option<usize>,
        span: Span,
    ) -> Eval<()> {
        // A `where` or `ensures` clause describes state rather than changing
        // it, and an obligation that had to be paid for in permissions is an
        // obligation people stop writing. So a contract does not contribute to
        // a row, which means reading one cannot break it either. That is a
        // decision rather than an oversight; see `design/03-effects.md`.
        if self.in_contract > 0 {
            return Ok(());
        }
        let Some(id) = self.effect_id(effect) else {
            return Ok(());
        };
        let Some(name) = self.check_row(&id, operation, answered_by) else {
            return Ok(());
        };
        let effect_name = &id.1;
        Err(self.fail(
            Diagnostic::error(
                codes::ROW_NOT_KEPT,
                self.file(),
                span,
                format!("this performs `{effect_name}.{operation}`, and `{name}` is running and did not declare it"),
            )
            .with_primary_label("performed here")
            .with_note(
                "the file was accepted, so this is a hole in the effect checker rather than a mistake in the program; please report it",
            ),
        ))
    }

    /// Refuses to go any deeper, rather than letting the host stack decide.
    ///
    /// `Diverge` in a row says a function may not return. It does not make one
    /// return, so something has to answer for the case where it does not, and
    /// a runner that can be taken down by the program it is running is a runner
    /// nobody can point at an unfamiliar file.
    ///
    /// `file` is the caller's rather than `self.file()`. The span is the call,
    /// and by the time this runs the current module is the callee's, so a
    /// recursion that crosses a module boundary used to be reported at the
    /// caller's byte offsets inside the callee's file.
    fn too_deep(&self, file: FileId, span: Span) -> Eval<()> {
        if self.frames.len() <= MAX_DEPTH {
            return Ok(());
        }
        Err(self.fail(
            Diagnostic::error(
                codes::TOO_DEEP,
                file,
                span,
                format!("this call went more than {MAX_DEPTH} deep"),
            )
            .with_primary_label("too deep")
            .with_note(
                "a function that can reach itself has to declare `Diverge`, and declaring it does not make it stop",
            ),
        ))
    }

    /// The part of a call that runs inside the new frame.
    ///
    /// `captures` is whether anything here could read what entering captures.
    /// A function with no `ensures` and no `old` or `unchanged` in it has
    /// nothing to snapshot for, and snapshotting anyway copied every installed
    /// handler's state on every call in a program that installs one.
    ///
    /// `call_file` is the caller's. See [`Interp::too_deep`]: a precondition
    /// failure is the caller's bug and points at the call, so it has to be
    /// filed against the file that call was written in.
    fn call_body(
        &mut self,
        function: &'a FnDecl,
        call_span: Span,
        call_file: FileId,
        captures: bool,
    ) -> Eval<Value> {
        // Preconditions first. A failure here is the caller's fault, so the
        // diagnostic points at the call and only mentions the clause.
        for requirement in &function.contract.requires {
            if !self.in_a_contract(|me| me.condition(requirement))? {
                let name = function.sig.name.name.clone();
                let diagnostic = Diagnostic::error(
                    codes::PRECONDITION_FAILED,
                    call_file,
                    call_span,
                    format!("this call does not satisfy what `{name}` requires"),
                )
                .with_primary_label("precondition not met")
                .with_note("a precondition failure is a bug in the caller")
                // The clause is in the callee and the call is in the caller,
                // and this is filed against the caller because that is whose
                // bug it is. So the label says which file it means. This used
                // to be dropped whenever the two differed, because a label
                // carried a span and no file and would have landed on
                // whatever happened to sit at those byte offsets in the wrong
                // one.
                .with_secondary_in(
                    self.file(),
                    requirement.span(),
                    "the clause that failed",
                );
                return Err(self.fail(diagnostic));
            }
        }

        if captures {
            self.in_a_contract(|me| me.capture_entry_state(&function.contract.ensures))?;
        }

        let outcome = self.eval_block(&function.body);
        let value = match outcome {
            Ok(value) => value,
            Err(Signal::Return(value)) => value,
            Err(other) => {
                if captures {
                    self.olds.pop();
                    self.entry_states.pop();
                }
                return Err(other);
            }
        };

        let obligations =
            self.in_a_contract(|me| me.check_ensures(function, &value, call_span, call_file));
        if captures {
            self.olds.pop();
            self.entry_states.pop();
        }
        obligations?;

        Ok(value)
    }

    /// Evaluates every `old(...)` and snapshots handler state, before the body
    /// gets a chance to change anything.
    fn capture_entry_state(&mut self, ensures: &'a [Ensures]) -> Eval<()> {
        let mut olds = HashMap::new();
        let mut targets = Vec::new();
        for obligation in ensures {
            collect_olds(&obligation.condition, &mut targets);
        }

        self.olds.push(HashMap::new());
        for (span, inner) in targets {
            match self.eval(inner) {
                Ok(value) => {
                    olds.insert(span, value);
                }
                Err(signal) => {
                    self.olds.pop();
                    return Err(signal);
                }
            }
        }
        self.olds.pop();
        self.olds.push(olds);

        let mut states = HashMap::new();
        for instance in &self.handlers {
            states.insert(instance.effect.clone(), instance.state.clone());
        }
        self.entry_states.push(states);

        Ok(())
    }

    fn check_ensures(
        &mut self,
        function: &'a FnDecl,
        value: &Value,
        call_span: Span,
        call_file: FileId,
    ) -> Eval<()> {
        // The outcome is whatever the function actually produced. A function
        // that does not return a `Result` cannot fail, so everything it does is
        // an `ok` outcome.
        let outcome = match value {
            Value::Result { ok: false, .. } => Outcome::Err,
            _ => Outcome::Ok,
        };

        for obligation in &function.contract.ensures {
            if obligation.outcome != outcome {
                continue;
            }

            // `result` is what the function produced: the success value for an
            // `ok` clause, the error value for an `err` one.
            if let Some(def) = result_def(&obligation.condition, self.resolutions()) {
                let bound = match (value, outcome) {
                    (Value::Result { value, .. }, _) => (**value).clone(),
                    (other, _) => other.clone(),
                };
                self.frame().insert(def, bound);
            }

            if !self.condition(&obligation.condition)? {
                let name = function.sig.name.name.clone();
                let diagnostic = Diagnostic::error(
                    codes::POSTCONDITION_FAILED,
                    self.file(),
                    obligation.span,
                    format!("`{name}` did not keep this promise"),
                )
                .with_primary_label("postcondition not met")
                .with_note("a postcondition failure is a bug in the function, not in the caller")
                // The other direction: this is filed against the function,
                // because a broken promise is the function's bug, and the
                // call that caught it can be anywhere. For a library function
                // called from twenty places that label is the diagnosis.
                .with_secondary_in(call_file, call_span, "called from here");
                return Err(self.fail(diagnostic));
            }
        }
        Ok(())
    }

    /// The `Guarded` tier, actually guarding something.
    ///
    /// Reached from one place: after evaluating an expression the checker
    /// recorded an obligation for. Every way a refined value can come into
    /// existence goes through there, which is the point. The old arrangement
    /// checked arguments and annotated `let`s and nothing else, so a return
    /// value carried a warning and no check.
    fn guard(&mut self, guard: Guard, value: &Value, span: Span) -> Eval<()> {
        let refinement = guard.refinement;
        let Some(predicate) = self.refinement(refinement) else {
            return Ok(());
        };

        // The obligation is about the number inside the `ok`, so that is what
        // has to satisfy the predicate. An `err` carries no such number and
        // there is nothing to check.
        let value = if guard.inside_ok {
            match value.ok_payload() {
                Some(payload) => payload,
                None => return Ok(()),
            }
        } else {
            value.clone()
        };

        // The predicate talks about `value`, which is not in scope anywhere
        // else, so it is bound here.
        let started = self.profile.as_ref().map(|_| Instant::now());
        let passes = self.eval_predicate(refinement, predicate, &value)?;
        if let Some(started) = started {
            self.add_contract_time(started.elapsed());
        }
        if passes {
            return Ok(());
        }

        let name = self.refinement_name(refinement);
        Err(self.fail(
            Diagnostic::error(
                codes::REFINEMENT_FAILED,
                self.file(),
                span,
                format!("{value} does not satisfy `{name}`"),
            )
            .with_primary_label("violates the refinement")
            .with_secondary(predicate.span(), "the predicate it has to satisfy")
            .with_note("the compiler could not prove this statically, so it is checked here"),
        ))
    }

    fn refinement_name(&self, def: DefId) -> String {
        self.resolutions().def(def).name.clone()
    }

    /// Evaluates a refinement predicate with `value` standing for the thing
    /// being checked.
    ///
    /// The predicate is an ordinary expression and is evaluated as one, in a
    /// frame of its own holding nothing but `value`. It used to be walked by a
    /// small interpreter with its own idea of which operators exist, so
    /// `length(value) > 0` was unrunnable while `value > 0` was fine, for no
    /// reason anybody could have predicted from the language.
    fn eval_predicate(&mut self, alias: DefId, predicate: &'a Expr, value: &Value) -> Eval<bool> {
        let mut frame = Frame::default();
        if let Some(subject) = self.subject(alias) {
            frame.insert(subject, value.clone());
        }

        self.frames.push(frame);
        let held = self.condition(predicate);
        self.frames.pop();
        held
    }

    // -- construction ------------------------------------------------------

    fn build(&mut self, path: &'a Expr, fields: &'a [FieldInit], span: Span) -> Eval<Value> {
        let def = match path {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions().resolution(name.span),
            _ => None,
        };

        let mut values = Fields::new();
        for field in fields {
            let value = match &field.value {
                Some(value) => self.eval(value)?,
                None => self.read(&field.name)?,
            };
            values.insert(field.name.name.clone(), value);
        }

        match def.map(|def| (def, self.kind_of(def))) {
            Some((_, DefKind::Record)) => Ok(Value::record(values)),
            Some((def, _)) => match self.variant_id(def) {
                Some((origin, name)) => Ok(Value::variant(origin, name, values)),
                // A record from another module has no nominal identity at
                // runtime either way, since records already compare by their
                // contents alone.
                None if self.imported_record(def) => Ok(Value::record(values)),
                None => Err(self.not_runnable(span, "this literal")),
            },
            _ => Err(self.not_runnable(span, "this literal")),
        }
    }

    /// Whether an imported name is a record in the module it came from.
    fn imported_record(&self, def: DefId) -> bool {
        self.resolutions()
            .import(def)
            .is_some_and(|export| export.kind == ExportKind::Record)
    }

    fn install(&mut self, expr: &'a Expr) -> Eval<()> {
        let (path, fields): (&Expr, &[FieldInit]) = match expr {
            Expr::StructLit { path, fields, .. } => (path, fields),
            other => (other, &[]),
        };

        let def = match path {
            Expr::Ident(ident) => self.def_of(ident),
            _ => None,
        };
        let Some(def) = def else {
            return Err(self.not_runnable(expr.span(), "this handler"));
        };

        // A handler from another module is installed here and runs there, so
        // the instance remembers where its operations live.
        let Some((home, handler_def, declaration)) = self.handler_at(def) else {
            return Err(self.not_runnable(expr.span(), "this handler"));
        };

        let mut state = Fields::new();
        for field in fields {
            let value = match &field.value {
                Some(value) => self.eval(value)?,
                None => self.read(&field.name)?,
            };
            state.insert(field.name.name.clone(), value);
        }

        for field in &declaration.state {
            if !state.contains_key(&field.name.name) {
                let handler = declaration.name.name.clone();
                let missing = field.name.name.clone();
                return Err(self.fail(
                    Diagnostic::error(
                        codes::NOT_RUNNABLE,
                        self.file(),
                        expr.span(),
                        format!("`{handler}` needs an initial value for `{missing}`"),
                    )
                    .with_primary_label("incomplete handler")
                    .with_secondary(field.span, "declared here"),
                ));
            }
        }

        // The effect a handler implements is named where the handler is
        // written, so it is resolved in that module.
        let there = &self.modules[home];
        let Some(effect) = there
            .resolutions
            .resolution(declaration.effect.span)
            .and_then(|effect| self.effect_id_in(home, effect))
        else {
            return Err(self.not_runnable(expr.span(), "this handler's effect"));
        };

        self.handlers.push(Instance {
            handler: handler_def,
            module: home,
            effect,
            state,
        });
        Ok(())
    }

    /// The handler a name refers to, wherever it was declared.
    fn handler_at(&self, def: DefId) -> Option<(usize, DefId, &'a HandlerDecl)> {
        if self.kind_of(def) == DefKind::Handler {
            return Some((self.current, def, self.handler_decl(def)?));
        }
        if self.resolutions().import(def)?.kind != ExportKind::Handler {
            return None;
        }

        let module = self.resolutions().import_module(def)?;
        let index = *self.by_path.get(module)?;
        let name = &self.resolutions().def(def).name;
        let there = &self.modules[index];
        there
            .handler_decls
            .iter()
            .find(|(_, declaration)| declaration.name.name == *name)
            .map(|(id, declaration)| (index, *id, *declaration))
    }

    // -- statements --------------------------------------------------------

    fn eval_block(&mut self, block: &'a Block) -> Eval<Value> {
        for stmt in &block.stmts {
            self.exec(stmt)?;
        }
        match &block.tail {
            Some(tail) => self.eval(tail),
            None => Ok(Value::Unit),
        }
    }

    /// Runs the `finally` block of each handler installed at or after `base`,
    /// in reverse order (most recently installed first).
    ///
    /// `finally` always runs, regardless of whether the body succeeded,
    /// returned early, or failed. This is what makes it safe for a handler
    /// to acquire a resource: whatever ends the `with` block, the `finally`
    /// block will release it.
    ///
    /// When the incoming result is already a `Fail`, that failure is kept.
    /// If the body did not fail but a `finally` block fails, that failure is
    /// returned. A resource the body could see but `finally` could not clean
    /// up is still a problem worth reporting.
    fn run_finally_blocks(&mut self, base: usize, incoming: Eval<Value>) -> Eval<Value> {
        // Walk from the most recently installed handler back to `base`.
        let top = self.handlers.len();
        if base == top {
            return incoming;
        }

        // Whether the incoming outcome is already a hard failure. If it is,
        // any failure in a `finally` block is secondary and is dropped so the
        // original reason the body stopped is what the caller sees.
        let body_failed = matches!(incoming, Err(Signal::Fail(_)));

        let mut outcome = incoming;

        for index in (base..top).rev() {
            let home = self.handlers[index].module;
            let handler_def = self.handlers[index].handler;

            let Some(declaration) = self.modules[home].handler_decls.get(&handler_def).copied()
            else {
                continue;
            };
            let Some(finally) = &declaration.finally else {
                continue;
            };

            // Run the `finally` block in the handler's module with the
            // handler's state accessible. A fresh frame holds any bindings
            // made inside the block.
            let caller = self.current;
            self.current = home;
            self.frames.push(Frame::default());
            self.rows.push(RowFrame {
                handled: self.handlers.len(),
                handler: Some(index),
                promise: None,
            });
            self.inside_handler.push(index);

            let finally_result = self.eval_block(finally);

            self.inside_handler.pop();
            self.rows.pop();
            self.frames.pop();
            self.current = caller;

            // Keep the original failure; only replace a non-failing outcome
            // with a `finally` failure.
            if !body_failed {
                if let Err(signal) = finally_result {
                    outcome = Err(signal);
                }
            }
        }

        outcome
    }

    fn exec(&mut self, stmt: &'a Stmt) -> Eval<()> {
        match stmt {
            Stmt::Let {
                pattern,
                ty: _,
                init,
                ..
            } => {
                let value = self.eval(init)?;
                self.bind(&value, pattern);
                Ok(())
            }

            Stmt::Assign { target, value, .. } => {
                let value = self.eval(value)?;
                let Some(def) = self.def_of(target) else {
                    return Err(self.not_runnable(target.span, "this assignment"));
                };
                let Some(index) = self.inside_handler.last().copied() else {
                    return Err(self.not_runnable(target.span, "assignment from outside a handler"));
                };
                let name = self.state_name(def);
                self.handlers[index].state.insert(name, value);
                Ok(())
            }

            Stmt::Return { value, .. } => {
                let value = match value {
                    Some(value) => self.eval(value)?,
                    None => Value::Unit,
                };
                Err(Signal::Return(value))
            }

            // `abandon` unwinds the computation unconditionally.
            //
            // The abandoned computation does not receive a return value from
            // the effect operation; instead the stack unwinds through any
            // `finally` clauses on `with` blocks. `assert refuses` cannot
            // catch this because DEED6011 is not a contract failure.
            Stmt::Abandon { span } => Err(self.fail(
                Diagnostic::error(
                    codes::ABANDONED,
                    self.file(),
                    *span,
                    "this computation was abandoned by its handler",
                )
                .with_primary_label("abandoned here"),
            )),

            Stmt::Assert { condition, span } => {
                if self.condition(condition)? {
                    return Ok(());
                }
                Err(self.assertion_failed(condition, *span))
            }

            // The one place anything is caught, and it catches one thing.
            //
            // A contract failure ends the run, which meant the `Guarded` tier
            // was the one thing a Deed test could not reach: a file of examples
            // showing a guard refusing something could not pass. Once
            // preconditions were read at the call site the checker refused the
            // file outright, so the better the checking got the further out of
            // reach the check itself went.
            //
            // Contracts only. Overflow, a missing handler and a run that went
            // too deep are not contracts and are not caught, because catching
            // them would be a `try` with a small vocabulary rather than a
            // statement about what a signature promised.
            Stmt::Refuses { subject, span } => match self.eval(subject) {
                Err(Signal::Fail(diagnostic)) if is_contract_failure(&diagnostic) => Ok(()),
                Err(other) => Err(other),
                Ok(value) => Err(self.fail(
                    Diagnostic::error(
                        codes::ASSERTION_FAILED,
                        self.file(),
                        *span,
                        format!("this was supposed to break a contract, and it produced {value}"),
                    )
                    .with_primary_label("nothing refused it")
                    .with_note(
                        "`assert refuses` passes when a `where` clause, an `ensures` clause or a refinement turns the value down, and nothing else counts",
                    ),
                )),
            },

            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(())
            }
        }
    }

    /// A failed assertion, with both sides shown when the condition compares
    /// two things. "assertion failed" on its own sends the reader back to run
    /// it again by hand, which is the round trip this whole design is about.
    fn assertion_failed(&mut self, condition: &'a Expr, span: Span) -> Signal {
        let detail = match condition {
            Expr::Binary {
                op:
                    op @ (BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge),
                lhs,
                rhs,
                ..
            } => match (self.eval(lhs), self.eval(rhs)) {
                (Ok(left), Ok(right)) => Some(format!(
                    "left is {left}, right is {right}, and `{}` is false",
                    op.as_str()
                )),
                _ => None,
            },
            _ => None,
        };

        let mut diagnostic = Diagnostic::error(
            codes::ASSERTION_FAILED,
            self.file(),
            condition.span(),
            "this assertion is not true",
        )
        .with_primary_label("evaluated to false")
        .with_secondary(span, "in this assert");

        if let Some(detail) = detail {
            diagnostic = diagnostic.with_note(detail);
        }

        self.fail(diagnostic)
    }

    // -- patterns ----------------------------------------------------------

    fn matches(&self, value: &Value, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Wildcard(_) => true,
            Pattern::Int { value: n, .. } => value.as_int() == Some(*n),
            Pattern::Bool { value: b, .. } => value.as_bool() == Some(*b),
            Pattern::Str { value: s, .. } => match value {
                Value::Str(actual) => &**actual == s.as_str(),
                _ => false,
            },
            Pattern::Path { segments, .. } => match segments.last() {
                Some(last) => match self.resolutions().resolution(last.span) {
                    Some(def) => match self.variant_id(def) {
                        Some(id) => match value {
                            Value::Variant(variant) => variant_is(variant, &id),
                            _ => false,
                        },
                        // A binding matches anything.
                        None => true,
                    },
                    None => true,
                },
                None => false,
            },
            Pattern::Record { path, .. } => match (path.last(), value) {
                (Some(last), Value::Variant(variant)) => self
                    .resolutions()
                    .resolution(last.span)
                    .and_then(|def| self.variant_id(def))
                    .is_some_and(|id| variant_is(variant, &id)),
                _ => false,
            },
            Pattern::Tuple { path, .. } => match value {
                Value::Result { ok, .. } => match self.builtin_name(path) {
                    Some(name) => (name == "ok") == *ok,
                    None => false,
                },
                _ => false,
            },
            Pattern::OneOf { alternatives, .. } => alternatives
                .iter()
                .any(|alternative| self.matches(value, alternative)),
            Pattern::Error(_) => false,
        }
    }

    /// The prelude name a pattern head refers to, if it is one.
    fn builtin_name(&self, path: &[Ident]) -> Option<String> {
        let last = path.last()?;
        let def = self.resolutions().resolution(last.span)?;
        (self.kind_of(def) == DefKind::Builtin).then(|| self.resolutions().def(def).name.clone())
    }

    fn bind(&mut self, value: &Value, pattern: &Pattern) {
        match pattern {
            Pattern::Tuple { elements, .. } => {
                let Value::Result { value: inner, .. } = value else {
                    return;
                };
                let inner = (**inner).clone();
                for element in elements {
                    self.bind(&inner, element);
                }
            }
            Pattern::Path { segments, .. } => {
                if let Some(only) = segments.first()
                    && segments.len() == 1
                    && let Some(def) = self.def_of(only)
                    && self.kind_of(def) == DefKind::Local
                {
                    self.frame().insert(def, value.clone());
                }
            }
            Pattern::Record { fields, .. } => {
                let Value::Variant(variant) = value else {
                    return;
                };
                for field in fields {
                    let Some(inner) = variant.fields.get(&field.name.name).cloned() else {
                        continue;
                    };
                    match &field.pattern {
                        Some(pattern) => self.bind(&inner, pattern),
                        None => {
                            if let Some(def) = self.def_of(&field.name) {
                                self.frame().insert(def, inner);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// What `trim` takes off the ends.
///
/// Written out rather than deferred to `char::is_whitespace`, so that what the
/// function does is as long as its documentation and no longer. The Unicode
/// whitespace table is a large amount of behaviour to hide behind a four
/// letter name, and a program that needs it can say so in a name of its own.
const WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// `split(text, separator)`, in characters rather than bytes.
///
/// An empty separator gives the characters. That is not a special case looking
/// for a home: the alternatives are an error the return type cannot express,
/// or an empty string between every pair of characters, which is what most
/// languages do and nobody wants. It also means walking a string needs no
/// second name in the prelude.
fn split(text: &str, separator: &str) -> Vec<Value> {
    if separator.is_empty() {
        return text.chars().map(|c| Value::str(c.to_string())).collect();
    }
    text.split(separator).map(Value::str).collect()
}

/// The definition `result` refers to inside an obligation, if it is used.
fn result_def(expr: &Expr, resolutions: &Resolutions) -> Option<DefId> {
    match expr {
        Expr::Ident(ident) if ident.name == "result" => resolutions.resolution(ident.span),
        Expr::Field { receiver, .. } => result_def(receiver, resolutions),
        Expr::Call { callee, args, .. } => result_def(callee, resolutions)
            .or_else(|| args.iter().find_map(|arg| result_def(arg, resolutions))),
        Expr::StructLit { path, fields, .. } => result_def(path, resolutions).or_else(|| {
            fields
                .iter()
                .filter_map(|field| field.value.as_ref())
                .find_map(|value| result_def(value, resolutions))
        }),
        Expr::Unary { operand, .. } | Expr::Try { operand, .. } => result_def(operand, resolutions),
        Expr::Binary { lhs, rhs, .. } => {
            result_def(lhs, resolutions).or_else(|| result_def(rhs, resolutions))
        }
        Expr::Old { expr, .. } => result_def(expr, resolutions),
        _ => None,
    }
}

/// Every `old(...)` inside an expression, as its span and what it wraps.
fn collect_olds<'a>(expr: &'a Expr, out: &mut Vec<(Span, &'a Expr)>) {
    match expr {
        Expr::Old { expr: inner, span } => {
            out.push((*span, inner));
            collect_olds(inner, out);
        }
        Expr::Field { receiver, .. } => collect_olds(receiver, out),
        Expr::Call { callee, args, .. } => {
            collect_olds(callee, out);
            for arg in args {
                collect_olds(arg, out);
            }
        }
        Expr::StructLit { path, fields, .. } => {
            collect_olds(path, out);
            for field in fields {
                if let Some(value) = &field.value {
                    collect_olds(value, out);
                }
            }
        }
        Expr::Unary { operand, .. } => collect_olds(operand, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_olds(lhs, out);
            collect_olds(rhs, out);
        }
        Expr::Try { operand, .. } => collect_olds(operand, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::Interp;
    use crate::{DeclaredRows, Guards, Program};
    use deed_ast::Item;
    use deed_diagnostics::SourceMap;
    use deed_resolve::Universe;

    /// Runs the first `test` block and reads something off the interpreter
    /// that ran it, which is the only way to see a table nothing outside the
    /// crate can.
    fn ran<T>(source: &str, read: impl Fn(&Interp) -> T) -> T {
        let mut sources = SourceMap::new();
        let file = sources.add("test.deed", source);

        let lexed = deed_lexer::tokenize(file, sources.file(file).text());
        assert!(!lexed.has_errors(), "should lex cleanly");
        let parsed = deed_parser::parse(file, &lexed.tokens);
        assert!(!parsed.has_errors(), "should parse cleanly");
        let resolved = deed_resolve::resolve(file, &parsed.module, &Universe::new());
        assert!(!resolved.has_errors(), "should resolve cleanly");

        let mut program = Program::new();
        program.add(
            file,
            &parsed.module,
            &resolved.resolutions,
            Guards::new(),
            DeclaredRows::new(),
        );

        let module = program.module(file).expect("the module should be there");
        let body = module
            .items
            .iter()
            .find_map(|item| match item {
                Item::Test(test) => Some(&test.body),
                _ => None,
            })
            .expect("a test block");

        let mut interp = Interp::new(&program, file);
        assert!(interp.eval_block(body).is_ok(), "it should run");
        read(&interp)
    }

    /// How many closure bodies the run kept.
    fn closures(source: &str) -> usize {
        ran(source, |interp| interp.closures.len())
    }

    /// How many call plans the run worked out, across every module in it.
    fn plans(source: &str) -> usize {
        ran(source, |interp| {
            interp.modules.iter().map(|code| code.plans.len()).sum()
        })
    }

    /// The table holds closure expressions, so it is the size of the program
    /// and not of the work.
    ///
    /// It used to push on every evaluation, which is once per turn for a
    /// closure literal inside a `for` body and once per call for one inside a
    /// function that calls itself. Nothing in `examples/` writes either, so
    /// nothing noticed, and the entries are never read again: the value points
    /// at the first one.
    #[test]
    fn a_closure_written_once_is_kept_once() {
        let one_turn = closures(
            "module a\n\n\
             fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n\
             test \"t\" {\n\
             \x20   let total = for n in [1] with sum = 0 {\n\
             \x20       sum + apply(|x: Int| x + n, 1)\n\
             \x20   }\n\
             \x20   assert total > 0\n\
             }\n",
        );
        let many_turns = closures(
            "module a\n\n\
             fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n\
             test \"t\" {\n\
             \x20   let total = for n in [1, 2, 3, 4, 5, 6, 7, 8] with sum = 0 {\n\
             \x20       sum + apply(|x: Int| x + n, 1)\n\
             \x20   }\n\
             \x20   assert total > 0\n\
             }\n",
        );

        assert_eq!(one_turn, 1);
        assert_eq!(
            many_turns, one_turn,
            "eight turns over one closure literal should keep one closure"
        );
    }

    /// Two literals are two entries, or the fix would be sharing bodies that
    /// have nothing to do with each other.
    #[test]
    fn two_closures_written_are_two_kept() {
        let kept = closures(
            "module a\n\n\
             fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n\
             test \"t\" {\n\
             \x20   let a = apply(|x: Int| x + 1, 1)\n\
             \x20   let b = apply(|x: Int| x + 2, 1)\n\
             \x20   assert a + b > 0\n\
             }\n",
        );
        assert_eq!(kept, 2);
    }

    /// The same shape one layer down. A plan is a property of a declaration,
    /// so calling one a thousand times works one out once, and the whole point
    /// of having them is that a call stops paying for it.
    #[test]
    fn a_function_called_many_times_is_planned_once() {
        let once = plans(
            "module a\n\n\
             fn itself(n: Int) -> Int { n }\n\n\
             test \"t\" {\n\
             \x20   let total = for n in [1] with sum = 0 { sum + itself(n) }\n\
             \x20   assert total > 0\n\
             }\n",
        );
        let often = plans(
            "module a\n\n\
             fn itself(n: Int) -> Int { n }\n\n\
             test \"t\" {\n\
             \x20   let total = for n in [1, 2, 3, 4, 5, 6, 7, 8] with sum = 0 { sum + itself(n) }\n\
             \x20   assert total > 0\n\
             }\n",
        );

        assert_eq!(once, 1);
        assert_eq!(
            often, once,
            "eight calls to one declaration should plan it once"
        );
    }

    /// And a function nobody calls is never planned, which is why this is
    /// worked out on the way in rather than for every declaration up front.
    #[test]
    fn a_function_nobody_calls_is_never_planned() {
        let planned = plans(
            "module a\n\n\
             fn called(n: Int) -> Int { n }\n\n\
             fn never(n: Int) -> Int { n }\n\n\
             test \"t\" {\n\
             \x20   assert called(1) == 1\n\
             }\n",
        );
        assert_eq!(planned, 1);
    }
}
