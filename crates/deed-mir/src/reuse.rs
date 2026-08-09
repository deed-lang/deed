//! What a function does with each of its parameters.
//!
//! `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md` names
//! this as the one fact that is missing, and fixes the order: compute it and
//! be able to print it before anything acts on it. Nothing here changes what a
//! program does. It answers, per parameter:
//!
//! - may the value the function returns be, or reach, the storage that
//!   parameter arrived in;
//! - may that storage still be reachable afterwards from somewhere that is not
//!   the result.
//!
//! A parameter where both answers are no is one whose block a caller holding
//! the only reference could hand over. That is what the transformation will
//! read. It is not read yet.
//!
//! ## One-sided on purpose
//!
//! A wrong answer here is not a compile error or a trap, it is a write into a
//! block somebody else is still reading, in bounds, that no contract and no
//! test reliably catches. So every case this cannot see through answers
//! [`ParamUse::KEEPS`]: a call through a value, an operation a handler
//! answers, anything crossing to the host. Those answers are always safe and
//! sometimes slow, and the cheap direction has no signal, which is why
//! [`Summary::print`] exists.

use std::collections::HashMap;

use crate::{Block, Expr, FuncId, Function, Program, Stmt, runtime};

/// What one function does with one of its parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ParamUse {
    /// The returned value may be, or may reach, this parameter's storage.
    pub shared_with_result: bool,
    /// The storage may outlive the call by some route other than the result:
    /// a handler's state, the host, or a callee that keeps it.
    pub retained: bool,
}

impl ParamUse {
    /// The answer for a parameter nothing could be worked out about.
    pub const KEEPS: ParamUse = ParamUse {
        shared_with_result: true,
        retained: true,
    };

    /// The answer for a parameter whose storage the call is finished with.
    pub const RELEASES: ParamUse = ParamUse {
        shared_with_result: false,
        retained: false,
    };

    /// Whether the caller's only reference is free once the call returns.
    pub fn released(&self) -> bool {
        !self.shared_with_result && !self.retained
    }

    fn widen(&mut self, other: ParamUse) {
        self.shared_with_result |= other.shared_with_result;
        self.retained |= other.retained;
    }
}

/// What a function does with every parameter it has.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Summary {
    pub params: Vec<ParamUse>,
}

impl Summary {
    pub fn param(&self, index: usize) -> ParamUse {
        self.params.get(index).copied().unwrap_or(ParamUse::KEEPS)
    }
}

/// Every function's summary, by [`FuncId`].
///
/// Read as a least fixed point over the call graph, so a function that calls
/// itself converges rather than being refused for being recursive.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Summaries {
    of: Vec<Summary>,
}

impl Summaries {
    pub fn get(&self, func: FuncId) -> &Summary {
        &self.of[func.0]
    }

    /// One line per parameter that is not the default answer.
    ///
    /// The decision record asks for this by name: the cheap direction is
    /// silent, so a function that could have handed its storage over and did
    /// not looks exactly like one that could not.
    pub fn print(&self, program: &Program) -> String {
        let mut out = String::new();
        for (index, summary) in self.of.iter().enumerate() {
            let function = &program.functions[index];
            for (at, use_of) in summary.params.iter().enumerate() {
                let word = if use_of.released() {
                    "releases"
                } else if !use_of.retained {
                    "returns"
                } else if !use_of.shared_with_result {
                    "retains"
                } else {
                    "keeps"
                };
                out.push_str(&format!("{word:9} {}#{at}\n", function.name));
            }
        }
        out
    }
}

/// What every function in the program does with its parameters.
pub fn summarise(program: &Program) -> Summaries {
    let mut of: Vec<Summary> = program
        .functions
        .iter()
        .map(|f| Summary {
            // Only a boxed parameter has storage to share. A number is copied
            // into the call and the question does not arise.
            params: f
                .params
                .iter()
                .map(|ty| {
                    if ty.is_boxed() {
                        ParamUse::default()
                    } else {
                        ParamUse::RELEASES
                    }
                })
                .collect(),
        })
        .collect();

    // Two bits per parameter and nothing here ever clears one, so a round
    // that changes anything sets a bit that stays set. Counting the bits
    // bounds the rounds, which makes stopping a property of the lattice
    // rather than of the comparison below noticing.
    let bound: usize = of.iter().map(|s| s.params.len() * 2).sum::<usize>() + 1;
    for _ in 0..bound {
        let before = of.clone();
        for (index, function) in program.functions.iter().enumerate() {
            let found = Walk::new(program, function, &of).summarise();
            for (at, use_of) in found.params.iter().enumerate() {
                of[index].params[at].widen(*use_of);
            }
        }
        if of == before {
            break;
        }
    }

    Summaries { of }
}

/// Which parameters a value may have come from.
///
/// A set rather than one answer, because `if` joins two branches and a record
/// holds several fields at once.
type From = Vec<usize>;

fn union(into: &mut From, other: &From) {
    for &param in other {
        if !into.contains(&param) {
            into.push(param);
        }
    }
}

struct Walk<'a> {
    program: &'a Program,
    function: &'a Function,
    known: &'a [Summary],
    /// What each slot may hold, by parameter.
    slots: HashMap<usize, From>,
    found: Summary,
}

impl<'a> Walk<'a> {
    fn new(program: &'a Program, function: &'a Function, known: &'a [Summary]) -> Self {
        let mut slots = HashMap::new();
        for (at, ty) in function.params.iter().enumerate() {
            if ty.is_boxed() {
                slots.insert(at, vec![at]);
            }
        }
        Walk {
            program,
            function,
            known,
            slots,
            found: Summary {
                params: vec![ParamUse::default(); function.params.len()],
            },
        }
    }

    fn summarise(mut self) -> Summary {
        let value = self.block(&self.function.body.clone());
        self.returns(&value);
        self.found
    }

    fn returns(&mut self, value: &From) {
        // A function handing back a number is handing back nothing of the
        // block a list arrived in, whatever the walk below found on the way.
        if !self.function.ret.is_boxed() {
            return;
        }
        for &param in value {
            self.found.params[param].shared_with_result = true;
        }
    }

    fn retain(&mut self, value: &From) {
        for &param in value {
            self.found.params[param].retained = true;
        }
    }

    fn block(&mut self, block: &Block) -> From {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        self.expr(&block.value)
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { local, value } => {
                let from = self.expr(value);
                self.slots.insert(local.0, from);
            }
            Stmt::Discard(value) => {
                self.expr(value);
            }
            Stmt::Return { value } => {
                let from = self.expr(value);
                self.returns(&from);
            }
            Stmt::Fail { .. } => {}
            Stmt::While { condition, body } => {
                // Twice, because a slot written on one turn is read on the
                // next and one pass would miss the way round.
                for _ in 0..2 {
                    self.expr(condition);
                    for stmt in body {
                        self.stmt(stmt);
                    }
                }
            }
            Stmt::SetField { object, value, .. } => {
                self.expr(object);
                // The only write to something already built, and it is a
                // handler's state, which outlives this call.
                let from = self.expr(value);
                self.retain(&from);
            }
        }
    }

    fn expr(&mut self, expr: &Expr) -> From {
        match expr {
            Expr::Unit | Expr::Bool(_) | Expr::Int(_) | Expr::Str(_) => From::new(),

            Expr::Local(local) => self.slots.get(&local.0).cloned().unwrap_or_default(),

            Expr::Unary { operand, .. } => {
                self.expr(operand);
                From::new()
            }

            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
                From::new()
            }

            Expr::Call { func, args, .. } => {
                let summary = &self.known[func.0];
                let mut result = From::new();
                let mut retained = From::new();
                for (at, arg) in args.iter().enumerate() {
                    let from = self.expr(arg);
                    let use_of = summary.param(at);
                    if use_of.shared_with_result {
                        union(&mut result, &from);
                    }
                    if use_of.retained {
                        union(&mut retained, &from);
                    }
                }
                self.retain(&retained);
                result
            }

            // No name, so no summary. Everything handed over is answered
            // "keeps it", which is the whole reason this is one-sided.
            Expr::CallIndirect { callee, args, .. } => {
                let mut all = self.expr(callee);
                for arg in args {
                    let from = self.expr(arg);
                    union(&mut all, &from);
                }
                self.retain(&all);
                all
            }

            // A handler answers, and which handler is not knowable here.
            Expr::Perform { args, .. } => {
                let mut all = From::new();
                for arg in args {
                    let from = self.expr(arg);
                    union(&mut all, &from);
                }
                self.retain(&all);
                all
            }

            // Out of the program altogether.
            Expr::Host { args, .. } => {
                let mut all = From::new();
                for arg in args {
                    let from = self.expr(arg);
                    union(&mut all, &from);
                }
                self.retain(&all);
                all
            }

            Expr::Runtime { name, args, ret } => {
                let mut result = From::new();
                for (at, arg) in args.iter().enumerate() {
                    let from = self.expr(arg);
                    if ret.is_boxed() && helper_result_reaches(name, at).unwrap_or(true) {
                        union(&mut result, &from);
                    }
                }
                result
            }

            // New storage, holding what it was given.
            Expr::Make {
                layout,
                variant,
                fields,
            } => {
                let types: Vec<bool> = self.program.layout(*layout).variants[*variant]
                    .fields
                    .iter()
                    .map(|field| field.ty.is_boxed())
                    .collect();
                let mut all = From::new();
                for (at, field) in fields.iter().enumerate() {
                    let from = self.expr(field);
                    if types.get(at).copied().unwrap_or(true) {
                        union(&mut all, &from);
                    }
                }
                all
            }

            Expr::List { element, items } => {
                let mut all = From::new();
                for item in items {
                    let from = self.expr(item);
                    if element.is_boxed() {
                        union(&mut all, &from);
                    }
                }
                all
            }

            // Reading a field hands back something inside the aggregate.
            Expr::Field {
                value,
                layout,
                variant,
                field,
            } => {
                let from = self.expr(value);
                let boxed = self
                    .program
                    .layout(*layout)
                    .variants
                    .get(*variant)
                    .and_then(|variant| variant.fields.get(*field))
                    .map(|field| field.ty.is_boxed())
                    .unwrap_or(true);
                if boxed { from } else { From::new() }
            }

            Expr::ElementAt {
                list,
                index,
                element,
            } => {
                self.expr(index);
                let from = self.expr(list);
                if element.is_boxed() {
                    from
                } else {
                    From::new()
                }
            }

            Expr::Discriminant { value, .. } => {
                self.expr(value);
                From::new()
            }

            Expr::Hashed { value, .. } => {
                self.expr(value);
                From::new()
            }

            Expr::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                self.expr(condition);
                let mut all = self.block(then);
                let other = self.block(otherwise);
                union(&mut all, &other);
                all
            }

            Expr::Block(block) => self.block(block),

            Expr::Install {
                state,
                body,
                operations,
                ..
            } => {
                // The cell outlives the block, and an operation may write a
                // parameter into it. What the operations do is their own
                // summary; from here the state is something kept.
                let from = self.expr(state);
                self.retain(&from);
                let _ = operations;
                self.block(body)
            }
        }
    }
}

/// Whether the result of a runtime helper may reach its argument at `at`.
///
/// `None` for a name this does not know, which the caller reads as yes. The
/// difference is not decoration: the set is closed and this compiler publishes
/// it, so `crates/deed-driver/tests/reuse.rs` can ask that every name in it
/// gets an answer here. Folding "unknown" into "yes" would make dropping an
/// entry invisible.
///
/// None of them retain anything: the runtime keeps no state between calls, so
/// the only storage that outlives one is the storage it returns.
fn helper_result_reaches(name: &str, at: usize) -> Option<bool> {
    let reaches = match name {
        // Numbers and booleans out; nothing of the argument comes with them.
        runtime::STR_EQ
        | runtime::STR_CMP
        | runtime::STR_LEN
        | runtime::LIST_LEN
        | runtime::CONTRACT_FAILED => false,

        // A `Result`, which is an aggregate and so is boxed, holding a number
        // it read and a sentence it wrote.
        runtime::STR_TO_INT => false,

        // Fresh text, character by character.
        runtime::STR_CONCAT
        | runtime::STR_TRIM
        | runtime::STR_UPPER
        | runtime::STR_LOWER
        | runtime::INT_TO_STR => false,

        // A fresh list of fresh pieces.
        runtime::STR_SPLIT => false,

        // Fresh text built from the pieces.
        runtime::STR_JOIN => false,

        // A fresh spine. The elements copied into it are the argument's, so
        // the answer is yes for a list of boxed things and this does not know
        // the element type: yes is the safe half.
        runtime::LIST_PUSH | runtime::LIST_ROOM_FROM | runtime::LIST_REPEAT => true,

        // Room for a length, holding nothing yet.
        runtime::LIST_NEW | runtime::LIST_ROOM => false,

        // The same list, one longer, written where it stands.
        runtime::LIST_APPEND => true,

        // An element, which lives inside the list.
        runtime::LIST_AT => at == 0,

        _ => return None,
    };
    Some(reaches)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every helper the runtime publishes has an answer written down.
    ///
    /// The set is closed, so "this one was never considered" is a thing that
    /// can be checked rather than a thing somebody notices later. Without
    /// this, dropping an entry costs nothing: the caller reads a missing one
    /// as yes, which is the safe half, and the analysis quietly gets worse.
    #[test]
    fn every_helper_the_runtime_publishes_has_an_answer() {
        let missing: Vec<&str> = runtime::ALL
            .iter()
            .copied()
            .filter(|name| helper_result_reaches(name, 0).is_none())
            .collect();
        assert!(missing.is_empty(), "no answer written down for {missing:?}");
    }

    /// And a name from outside that set is not quietly given one.
    #[test]
    fn a_name_the_runtime_does_not_publish_has_none() {
        assert_eq!(helper_result_reaches("deed_rt_invented", 0), None);
    }

    /// The list is the first argument of `at`, and the index is the second.
    #[test]
    fn an_element_comes_from_the_list_and_not_the_index() {
        assert_eq!(helper_result_reaches(runtime::LIST_AT, 0), Some(true));
        assert_eq!(helper_result_reaches(runtime::LIST_AT, 1), Some(false));
    }
}
