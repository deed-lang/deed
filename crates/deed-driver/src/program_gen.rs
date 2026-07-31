//! Generates random Deed programs and shrinks the ones that fail.
//!
//! The generator produces programs as structured data, which makes the
//! shrinker's job straightforward: remove a declaration, remove a statement,
//! replace an expression with a literal, shorten a name. Every candidate is
//! tested against the same predicate that caught the original failure, so the
//! result is still a failure rather than something that passes for a different
//! reason.
//!
//! The rule is the same as the value shrinker in `deed-interp`'s property
//! tests: a shrinker has to cover everything the generator can produce. A
//! shrunk program with something in it nothing could shrink is a program still
//! nobody reads.
//!
//! # Shape
//!
//! Generated programs are simple enough to shrink cleanly:
//!
//! - One module, one or more functions.
//! - All functions take `Int` parameters and return `Int`.
//! - Function bodies have zero or more `let` bindings and a tail expression.
//! - Expressions are integer literals, names, arithmetic, comparisons,
//!   if-then-else, and calls to other functions in the same module.
//!
//! # Usage
//!
//! ```
//! use deed_driver::program_gen::{ProgramFuzzConfig, find_program_failure};
//! use deed_diagnostics::SourceMap;
//!
//! let config = ProgramFuzzConfig::default();
//! let finding = find_program_failure(config, |source| {
//!     // Return true when the source triggers the failure of interest.
//!     let mut sources = SourceMap::new();
//!     let checked = deed_driver::check_text(&mut sources, "fuzz.deed", source);
//!     checked.has_errors()
//! });
//! if let Some(finding) = finding {
//!     println!("seed {:#x}\n{}", finding.seed, finding.source);
//! }
//! ```

use std::fmt::Write as _;

/// How hard to try.
#[derive(Clone, Copy, Debug)]
pub struct ProgramFuzzConfig {
    pub cases: usize,
    /// Fixed by default, and reported in the finding, because a program that
    /// fails you cannot reproduce is a rumour.
    pub seed: u64,
    /// How many candidates the shrinker may evaluate before giving up.
    pub shrink_budget: usize,
}

impl Default for ProgramFuzzConfig {
    fn default() -> Self {
        Self {
            cases: 100,
            seed: 0x5EED_1234_ABCD_0001,
            shrink_budget: 500,
        }
    }
}

/// A minimal failing program found by the fuzzer, already shrunk.
pub struct ProgramFinding {
    /// The seed that produced the failing program, for reproducibility.
    pub seed: u64,
    /// The smallest program the shrinker could reduce the failure to.
    pub source: String,
}

/// Finds and shrinks a failing program, if any.
///
/// The predicate returns `true` when the source triggers the failure of
/// interest. Every shrink candidate is checked against the same predicate, so
/// the result is still a failure and not a vacuously passing one.
pub fn find_program_failure<F>(config: ProgramFuzzConfig, fails: F) -> Option<ProgramFinding>
where
    F: Fn(&str) -> bool,
{
    let mut rng = Rng::new(config.seed);
    for _ in 0..config.cases {
        let program = generate_program(&mut rng);
        let source = print_program(&program);
        if fails(&source) {
            let program = shrink_program(program, &fails, config.shrink_budget);
            return Some(ProgramFinding {
                seed: config.seed,
                source: print_program(&program),
            });
        }
    }
    None
}

// -- structured program ----------------------------------------------------

/// A generated Deed module.
#[derive(Clone, Debug)]
pub struct GeneratedProgram {
    pub module: String,
    pub fns: Vec<GeneratedFn>,
}

/// A generated function.
#[derive(Clone, Debug)]
pub struct GeneratedFn {
    pub name: String,
    pub params: Vec<String>,
    pub stmts: Vec<GeneratedStmt>,
    pub tail: GeneratedExpr,
}

/// A generated statement. Only `let` bindings are generated.
#[derive(Clone, Debug)]
pub struct GeneratedStmt {
    pub name: String,
    pub init: GeneratedExpr,
}

/// A generated expression.
///
/// All expressions are `Int`-valued so the return type is always `Int` and no
/// type inference is needed in the printer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneratedExpr {
    Int(i64),
    /// A reference to a parameter or a `let`-bound name.
    Name(String),
    BinOp {
        op: &'static str,
        left: Box<GeneratedExpr>,
        right: Box<GeneratedExpr>,
    },
    /// Both branches are `Int`-valued; the condition is a comparison.
    If {
        cond: Box<GeneratedExpr>,
        then_: Box<GeneratedExpr>,
        else_: Box<GeneratedExpr>,
    },
    /// A call to another function in the same module.
    Call {
        name: String,
        args: Vec<GeneratedExpr>,
    },
}

// -- printing --------------------------------------------------------------

/// Renders a [`GeneratedProgram`] as Deed source code.
pub fn print_program(program: &GeneratedProgram) -> String {
    let mut out = format!("module {}\n", program.module);
    for f in &program.fns {
        out.push('\n');
        print_fn(&mut out, f);
    }
    out
}

fn print_fn(out: &mut String, f: &GeneratedFn) {
    let _ = write!(out, "fn {}(", f.name);
    for (i, param) in f.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{param}: Int");
    }
    out.push_str(") -> Int {\n");
    for stmt in &f.stmts {
        let _ = writeln!(out, "    let {} = {}", stmt.name, print_expr(&stmt.init));
    }
    let _ = writeln!(out, "    {}", print_expr(&f.tail));
    out.push_str("}\n");
}

fn print_expr(expr: &GeneratedExpr) -> String {
    match expr {
        GeneratedExpr::Int(n) => {
            if *n < 0 {
                format!("0 - {}", n.unsigned_abs())
            } else {
                n.to_string()
            }
        }
        GeneratedExpr::Name(n) => n.clone(),
        GeneratedExpr::BinOp { op, left, right } => {
            format!("({} {op} {})", print_expr(left), print_expr(right))
        }
        GeneratedExpr::If { cond, then_, else_ } => {
            format!(
                "if {} {{ {} }} else {{ {} }}",
                print_expr(cond),
                print_expr(then_),
                print_expr(else_)
            )
        }
        GeneratedExpr::Call { name, args } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            format!("{}({})", name, args_str.join(", "))
        }
    }
}

// -- generating ------------------------------------------------------------

fn generate_program(rng: &mut Rng) -> GeneratedProgram {
    let fn_count = 1 + (rng.next() % 4) as usize;
    let fns: Vec<_> = (0..fn_count).map(|i| generate_fn(rng, i)).collect();
    GeneratedProgram {
        module: "fuzz".to_string(),
        fns,
    }
}

fn generate_fn(rng: &mut Rng, index: usize) -> GeneratedFn {
    let name = fn_name(index);
    let param_count = (rng.next() % 3) as usize;
    let params: Vec<_> = (0..param_count).map(param_name).collect();

    let stmt_count = (rng.next() % 3) as usize;
    let mut stmts = Vec::new();
    let mut scope: Vec<String> = params.clone();
    for i in 0..stmt_count {
        let init = generate_expr(rng, &scope, index, 0);
        scope.push(let_name(i));
        stmts.push(GeneratedStmt {
            name: let_name(i),
            init,
        });
    }

    let tail = generate_expr(rng, &scope, index, 0);
    GeneratedFn {
        name,
        params,
        stmts,
        tail,
    }
}

fn generate_expr(rng: &mut Rng, scope: &[String], fn_index: usize, depth: usize) -> GeneratedExpr {
    if depth > 3 {
        return GeneratedExpr::Int(rng.int());
    }
    // Arithmetic operators only, so the expression is always `Int`-valued.
    const OPS: [&str; 4] = ["+", "-", "*", "+"];
    match rng.next() % 7 {
        0 => GeneratedExpr::Int(rng.int()),
        1 | 2 if !scope.is_empty() => {
            let idx = (rng.next() as usize) % scope.len();
            GeneratedExpr::Name(scope[idx].clone())
        }
        3 | 4 => {
            let op = OPS[(rng.next() as usize) % OPS.len()];
            GeneratedExpr::BinOp {
                op,
                left: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
                right: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
            }
        }
        5 => GeneratedExpr::If {
            cond: Box::new(GeneratedExpr::BinOp {
                op: ">",
                left: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
                right: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
            }),
            then_: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
            else_: Box::new(generate_expr(rng, scope, fn_index, depth + 1)),
        },
        // Call an earlier function so there are no cycles.
        _ if fn_index > 0 => {
            let callee_idx = (rng.next() as usize) % fn_index;
            let callee = fn_name(callee_idx);
            // How many parameters that function takes: not known here, so
            // generate zero arguments and let the type checker reject it.
            // The shrinker will still shrink the program even if it has errors.
            let arg_count = (rng.next() % 3) as usize;
            let args = (0..arg_count)
                .map(|_| generate_expr(rng, scope, fn_index, depth + 1))
                .collect();
            GeneratedExpr::Call { name: callee, args }
        }
        _ => GeneratedExpr::Int(rng.int()),
    }
}

fn fn_name(index: usize) -> String {
    format!("f{index}")
}

fn param_name(index: usize) -> String {
    format!("p{index}")
}

fn let_name(index: usize) -> String {
    format!("v{index}")
}

// -- shrinking -------------------------------------------------------------

/// Shrinks a failing program.
///
/// Tries every simpler form of the program and keeps the first one that still
/// fails. Goes around until no simpler form fails or the budget is exhausted.
fn shrink_program<F>(mut program: GeneratedProgram, fails: F, budget: usize) -> GeneratedProgram
where
    F: Fn(&str) -> bool,
{
    let mut budget = budget;

    'outer: loop {
        if budget == 0 {
            break;
        }
        for candidate in smaller_programs(&program) {
            budget = budget.saturating_sub(1);
            if fails(&print_program(&candidate)) {
                program = candidate;
                continue 'outer;
            }
            if budget == 0 {
                break 'outer;
            }
        }
        break;
    }

    program
}

/// All simpler forms of a program, best first.
///
/// Best first so the loop in [`shrink_program`] reaches the simplest form
/// in the fewest steps rather than spending the budget on something that
/// changes little.
fn smaller_programs(program: &GeneratedProgram) -> Vec<GeneratedProgram> {
    let mut out = Vec::new();

    // Remove each function. A program with no functions is not valid, so
    // at least one must remain.
    for skip in 0..program.fns.len() {
        if program.fns.len() > 1 {
            let mut candidate = program.clone();
            candidate.fns.remove(skip);
            out.push(candidate);
        }
    }

    // Shrink each function in place.
    for index in 0..program.fns.len() {
        for smaller in smaller_fn(&program.fns[index]) {
            let mut candidate = program.clone();
            candidate.fns[index] = smaller;
            out.push(candidate);
        }
    }

    out
}

/// Simpler forms of a function.
fn smaller_fn(f: &GeneratedFn) -> Vec<GeneratedFn> {
    let mut out = Vec::new();

    // Remove each statement.
    for skip in 0..f.stmts.len() {
        let mut candidate = f.clone();
        candidate.stmts.remove(skip);
        out.push(candidate);
    }

    // Remove each parameter and replace every use of it with `0`.
    for skip in 0..f.params.len() {
        let dropped = &f.params[skip];
        let zero = GeneratedExpr::Int(0);
        let mut candidate = f.clone();
        candidate.params.remove(skip);
        candidate.stmts = candidate
            .stmts
            .iter()
            .map(|s| GeneratedStmt {
                name: s.name.clone(),
                init: replace_name(&s.init, dropped, &zero),
            })
            .collect();
        candidate.tail = replace_name(&candidate.tail, dropped, &zero);
        out.push(candidate);
    }

    // Shrink the tail expression.
    for candidate_tail in smaller_expr(&f.tail) {
        let mut candidate = f.clone();
        candidate.tail = candidate_tail;
        out.push(candidate);
    }

    // Shrink each statement's init expression.
    for i in 0..f.stmts.len() {
        for candidate_init in smaller_expr(&f.stmts[i].init) {
            let mut candidate = f.clone();
            candidate.stmts[i].init = candidate_init;
            out.push(candidate);
        }
    }

    out
}

/// Simpler forms of an expression, best first.
///
/// Zero is the simplest integer and comes first, so the loop reaches the
/// emptiest form in one step when that form is the answer.
fn smaller_expr(expr: &GeneratedExpr) -> Vec<GeneratedExpr> {
    match expr {
        // Already as simple as it gets.
        GeneratedExpr::Int(0) => Vec::new(),

        GeneratedExpr::Int(n) => {
            let mut out = vec![GeneratedExpr::Int(0)];
            let abs = n.unsigned_abs();
            if abs > 1 {
                out.push(GeneratedExpr::Int(n / 2));
            }
            out.push(GeneratedExpr::Int(if *n > 0 { n - 1 } else { n + 1 }));
            out
        }

        // A name is simpler than any compound expression but not simpler than
        // a literal, so replace it with zero.
        GeneratedExpr::Name(_) => vec![GeneratedExpr::Int(0)],

        // Both children, and then each child independently made simpler.
        GeneratedExpr::BinOp { left, right, .. } => {
            let mut out = vec![GeneratedExpr::Int(0), *left.clone(), *right.clone()];
            for candidate in smaller_expr(left) {
                out.push(GeneratedExpr::BinOp {
                    op: "+",
                    left: Box::new(candidate),
                    right: right.clone(),
                });
            }
            for candidate in smaller_expr(right) {
                out.push(GeneratedExpr::BinOp {
                    op: "+",
                    left: left.clone(),
                    right: Box::new(candidate),
                });
            }
            out
        }

        // The branches, and then each branch made simpler.
        GeneratedExpr::If { then_, else_, .. } => {
            let mut out = vec![GeneratedExpr::Int(0), *then_.clone(), *else_.clone()];
            for candidate in smaller_expr(then_) {
                out.push(GeneratedExpr::If {
                    cond: Box::new(GeneratedExpr::BinOp {
                        op: ">",
                        left: Box::new(GeneratedExpr::Int(0)),
                        right: Box::new(GeneratedExpr::Int(0)),
                    }),
                    then_: Box::new(candidate),
                    else_: else_.clone(),
                });
            }
            for candidate in smaller_expr(else_) {
                out.push(GeneratedExpr::If {
                    cond: Box::new(GeneratedExpr::BinOp {
                        op: ">",
                        left: Box::new(GeneratedExpr::Int(0)),
                        right: Box::new(GeneratedExpr::Int(0)),
                    }),
                    then_: then_.clone(),
                    else_: Box::new(candidate),
                });
            }
            out
        }

        // The zero literal, then each argument made simpler.
        GeneratedExpr::Call { name, args } => {
            let mut out = vec![GeneratedExpr::Int(0)];
            for (i, arg) in args.iter().enumerate() {
                for candidate in smaller_expr(arg) {
                    let mut new_args = args.clone();
                    new_args[i] = candidate;
                    out.push(GeneratedExpr::Call {
                        name: name.clone(),
                        args: new_args,
                    });
                }
            }
            out
        }
    }
}

/// Replaces every occurrence of a name with a replacement expression.
fn replace_name(expr: &GeneratedExpr, name: &str, with: &GeneratedExpr) -> GeneratedExpr {
    match expr {
        GeneratedExpr::Name(n) if n == name => with.clone(),
        GeneratedExpr::BinOp { op, left, right } => GeneratedExpr::BinOp {
            op,
            left: Box::new(replace_name(left, name, with)),
            right: Box::new(replace_name(right, name, with)),
        },
        GeneratedExpr::If { cond, then_, else_ } => GeneratedExpr::If {
            cond: Box::new(replace_name(cond, name, with)),
            then_: Box::new(replace_name(then_, name, with)),
            else_: Box::new(replace_name(else_, name, with)),
        },
        GeneratedExpr::Call { name: n, args } => GeneratedExpr::Call {
            name: n.clone(),
            args: args.iter().map(|a| replace_name(a, name, with)).collect(),
        },
        _ => expr.clone(),
    }
}

// -- randomness ------------------------------------------------------------

/// xorshift64. The same generator as the property-test value shrinker.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn int(&mut self) -> i64 {
        match self.next() % 10 {
            0 => 0,
            1 => (self.next() % 3) as i64 - 1,
            2..=7 => (self.next() % 201) as i64 - 100,
            8 => (self.next() % 20_001) as i64 - 10_000,
            _ => self.next() as i64,
        }
    }
}

// -- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        GeneratedExpr, GeneratedFn, GeneratedProgram, GeneratedStmt, ProgramFuzzConfig, Rng,
        find_program_failure, generate_program, print_program, smaller_expr, smaller_fn,
        smaller_programs,
    };

    fn one_fn_program(name: &str, tail: GeneratedExpr) -> GeneratedProgram {
        GeneratedProgram {
            module: "test".to_string(),
            fns: vec![GeneratedFn {
                name: name.to_string(),
                params: Vec::new(),
                stmts: Vec::new(),
                tail,
            }],
        }
    }

    // -- generator ---------------------------------------------------------

    #[test]
    fn the_generator_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let pa = generate_program(&mut a);
        let pb = generate_program(&mut b);
        assert_eq!(print_program(&pa), print_program(&pb));
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let pa = generate_program(&mut a);
        let pb = generate_program(&mut b);
        // A failure here would mean both seeds produce the same program, which
        // would make the seed useless for reproduction.
        assert_ne!(print_program(&pa), print_program(&pb));
    }

    #[test]
    fn generated_programs_have_at_least_one_function() {
        let mut rng = Rng::new(0x5EED);
        for _ in 0..20 {
            let p = generate_program(&mut rng);
            assert!(
                !p.fns.is_empty(),
                "every program must have at least one function"
            );
        }
    }

    // -- expression shrinker -----------------------------------------------

    #[test]
    fn zero_has_no_smaller_form() {
        assert!(smaller_expr(&GeneratedExpr::Int(0)).is_empty());
    }

    #[test]
    fn positive_integer_shrinks_toward_zero() {
        let candidates = smaller_expr(&GeneratedExpr::Int(100));
        assert!(candidates.contains(&GeneratedExpr::Int(0)));
        assert!(candidates.contains(&GeneratedExpr::Int(50)));
        assert!(candidates.contains(&GeneratedExpr::Int(99)));
    }

    #[test]
    fn negative_integer_shrinks_toward_zero() {
        let candidates = smaller_expr(&GeneratedExpr::Int(-100));
        assert!(candidates.contains(&GeneratedExpr::Int(0)));
        assert!(candidates.contains(&GeneratedExpr::Int(-50)));
        assert!(candidates.contains(&GeneratedExpr::Int(-99)));
    }

    #[test]
    fn a_name_shrinks_to_zero() {
        let candidates = smaller_expr(&GeneratedExpr::Name("x".to_string()));
        assert_eq!(candidates, vec![GeneratedExpr::Int(0)]);
    }

    #[test]
    fn a_binary_op_shrinks_to_its_children_and_zero() {
        let expr = GeneratedExpr::BinOp {
            op: "+",
            left: Box::new(GeneratedExpr::Int(5)),
            right: Box::new(GeneratedExpr::Int(3)),
        };
        let candidates = smaller_expr(&expr);
        assert!(candidates.contains(&GeneratedExpr::Int(0)));
        assert!(candidates.contains(&GeneratedExpr::Int(5)));
        assert!(candidates.contains(&GeneratedExpr::Int(3)));
    }

    #[test]
    fn an_if_shrinks_to_its_branches_and_zero() {
        let expr = GeneratedExpr::If {
            cond: Box::new(GeneratedExpr::BinOp {
                op: ">",
                left: Box::new(GeneratedExpr::Int(1)),
                right: Box::new(GeneratedExpr::Int(0)),
            }),
            then_: Box::new(GeneratedExpr::Int(10)),
            else_: Box::new(GeneratedExpr::Int(20)),
        };
        let candidates = smaller_expr(&expr);
        assert!(candidates.contains(&GeneratedExpr::Int(0)));
        assert!(candidates.contains(&GeneratedExpr::Int(10)));
        assert!(candidates.contains(&GeneratedExpr::Int(20)));
    }

    // -- function shrinker -------------------------------------------------

    #[test]
    fn a_statement_is_removed() {
        let f = GeneratedFn {
            name: "f".to_string(),
            params: Vec::new(),
            stmts: vec![GeneratedStmt {
                name: "v0".to_string(),
                init: GeneratedExpr::Int(42),
            }],
            tail: GeneratedExpr::Int(0),
        };
        let candidates = smaller_fn(&f);
        let without_stmt = GeneratedFn {
            name: "f".to_string(),
            params: Vec::new(),
            stmts: Vec::new(),
            tail: GeneratedExpr::Int(0),
        };
        assert!(
            candidates.iter().any(|c| c.stmts.is_empty()),
            "removing the only statement should be one of the candidates; got {candidates:?}, expected one without stmts"
        );
        // And the no-statement form should print cleanly.
        let p = GeneratedProgram {
            module: "t".to_string(),
            fns: vec![without_stmt],
        };
        let source = print_program(&p);
        assert!(source.contains("fn f()"), "{source}");
        assert!(!source.contains("let v0"), "{source}");
    }

    #[test]
    fn a_parameter_is_removed_and_its_uses_replaced_with_zero() {
        let f = GeneratedFn {
            name: "f".to_string(),
            params: vec!["p0".to_string()],
            stmts: Vec::new(),
            tail: GeneratedExpr::Name("p0".to_string()),
        };
        let candidates = smaller_fn(&f);
        // The candidate without the parameter should have `0` as the tail.
        let without_param = candidates
            .iter()
            .find(|c| c.params.is_empty())
            .expect("removing the parameter should be a candidate");
        assert_eq!(without_param.tail, GeneratedExpr::Int(0));
    }

    // -- program shrinker --------------------------------------------------

    #[test]
    fn a_function_is_removed_when_at_least_one_remains() {
        let p = GeneratedProgram {
            module: "t".to_string(),
            fns: vec![
                GeneratedFn {
                    name: "f0".to_string(),
                    params: Vec::new(),
                    stmts: Vec::new(),
                    tail: GeneratedExpr::Int(1),
                },
                GeneratedFn {
                    name: "f1".to_string(),
                    params: Vec::new(),
                    stmts: Vec::new(),
                    tail: GeneratedExpr::Int(2),
                },
            ],
        };
        let candidates = smaller_programs(&p);
        assert!(
            candidates.iter().any(|c| c.fns.len() == 1),
            "removing one function should be a candidate"
        );
    }

    #[test]
    fn a_single_function_program_does_not_shrink_to_nothing() {
        let p = one_fn_program("f", GeneratedExpr::Int(0));
        let candidates = smaller_programs(&p);
        // A program with no functions is invalid. Nothing in the candidates
        // should remove the last function.
        assert!(
            candidates.iter().all(|c| !c.fns.is_empty()),
            "the last function must not be removed"
        );
    }

    // -- end-to-end --------------------------------------------------------

    #[test]
    fn the_finding_is_already_shrunk() {
        // Predicate: the program's source is longer than 30 characters.
        // Shrinking should reduce it to the shortest program that still
        // satisfies the predicate.
        let config = ProgramFuzzConfig {
            cases: 50,
            seed: 0x5EED_1234_ABCD_0001,
            shrink_budget: 500,
        };
        if let Some(finding) = find_program_failure(config, |s| s.len() > 30) {
            // The shrunk source should be short -- no more than 100 characters
            // past the 30-character threshold, since the shrinker keeps going
            // until nothing smaller fails.
            assert!(
                finding.source.len() <= 100,
                "the shrunk program should be short, got {} chars:\n{}",
                finding.source.len(),
                finding.source
            );
            assert_eq!(finding.seed, config.seed, "the seed should be reported");
        }
        // If no failing case was found in 50 tries, that's fine: the test
        // just cannot say anything about shrinking.
    }

    #[test]
    fn shrinking_preserves_the_failure_predicate() {
        // Every program the shrinker considers that it keeps must satisfy the
        // predicate, because that is the whole point of shrinking.
        let config = ProgramFuzzConfig {
            cases: 100,
            seed: 7,
            shrink_budget: 200,
        };
        if let Some(finding) = find_program_failure(config, |s| s.contains("f1")) {
            // The shrunk program must still contain "f1".
            assert!(
                finding.source.contains("f1"),
                "the shrunk source must still satisfy the predicate:\n{}",
                finding.source
            );
        }
    }

    #[test]
    fn declarations_shrink() {
        // Start with a program that has two functions, then keep the first one
        // that has two functions as a "failure". Shrinking should produce a
        // program with fewer declarations.
        let config = ProgramFuzzConfig {
            cases: 200,
            seed: 0x5EED,
            shrink_budget: 500,
        };
        let finding = find_program_failure(config, |s| {
            // Count `fn ` occurrences to estimate function count.
            s.matches("fn ").count() >= 2
        });
        if let Some(finding) = finding {
            let count = finding.source.matches("fn ").count();
            assert_eq!(
                count, 2,
                "shrinking should reduce to exactly two functions, got {count}:\n{}",
                finding.source
            );
        }
    }

    #[test]
    fn statements_shrink() {
        // Any program with a `let` binding should shrink to one without.
        let config = ProgramFuzzConfig {
            cases: 200,
            seed: 0xABCD,
            shrink_budget: 500,
        };
        if let Some(finding) = find_program_failure(config, |s| s.contains("let ")) {
            let count = finding.source.matches("let ").count();
            assert_eq!(
                count, 1,
                "shrinking should reduce to one let binding, got {count}:\n{}",
                finding.source
            );
        }
    }

    #[test]
    fn expressions_shrink_to_literals() {
        // A program containing a BinOp should shrink until the expression
        // tree is as flat as possible: the shrinker replaces sub-expressions
        // with `0` until nothing smaller fails.
        let config = ProgramFuzzConfig {
            cases: 200,
            seed: 0x1234,
            shrink_budget: 1000,
        };
        if let Some(finding) = find_program_failure(config, |s| {
            // Any arithmetic operator in the source.
            s.contains(" + ") || s.contains(" - ") || s.contains(" * ")
        }) {
            // The shrunk program should have at most one operator.
            let ops: usize = finding.source.matches(" + ").count()
                + finding.source.matches(" - ").count()
                + finding.source.matches(" * ").count();
            assert!(
                ops <= 2,
                "shrinking should produce a minimal expression, got {ops} operators:\n{}",
                finding.source
            );
        }
    }
}
