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

use std::collections::HashMap;

use vow_ast::{
    BinaryOp, Block, Ensures, Expr, FieldInit, FnDecl, HandlerDecl, Ident, Item, Module, Outcome,
    Pattern, Stmt, Type, UnaryOp,
};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions};

use crate::codes;
use crate::value::{Fields, Value};

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

/// Runs every `test` block in a module.
pub fn run_tests(file: FileId, module: &Module, resolutions: &Resolutions) -> Vec<TestOutcome> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Test(test) => Some(test),
            _ => None,
        })
        .map(|test| {
            let mut interp = Interp::new(file, module, resolutions);
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

/// Non-local control flow.
///
/// `Return` is ordinary, `Fail` ends the program. There is no catch, because
/// there is nothing in the language that catches.
enum Signal {
    Return(Value),
    Fail(Box<Diagnostic>),
}

type Eval<T> = Result<T, Signal>;

/// A handler installed by a `with` block.
struct Instance {
    handler: DefId,
    effect: DefId,
    state: Fields,
}

struct Interp<'a> {
    file: FileId,
    resolutions: &'a Resolutions,

    functions: HashMap<DefId, &'a FnDecl>,
    handler_decls: HashMap<DefId, &'a HandlerDecl>,
    /// Type alias definition to the predicate it refines by.
    refinements: HashMap<DefId, &'a Expr>,
    /// Handler state definition to the field name it stands for.
    state_names: HashMap<DefId, String>,
    variant_names: HashMap<DefId, String>,

    /// One per active call. Bindings are keyed by definition, which resolution
    /// already made unique, so blocks need no scopes of their own.
    frames: Vec<HashMap<DefId, Value>>,
    handlers: Vec<Instance>,
    /// Which handler instance the running operation belongs to, if any.
    inside_handler: Vec<usize>,

    /// Values of `old(...)` captured on entry to the running call.
    olds: Vec<HashMap<Span, Value>>,
    /// Handler state captured on entry, for `unchanged(...)`.
    entry_states: Vec<HashMap<DefId, Fields>>,
}

impl<'a> Interp<'a> {
    fn new(file: FileId, module: &'a Module, resolutions: &'a Resolutions) -> Self {
        let mut interp = Interp {
            file,
            resolutions,
            functions: HashMap::new(),
            handler_decls: HashMap::new(),
            refinements: HashMap::new(),
            state_names: HashMap::new(),
            variant_names: HashMap::new(),
            frames: vec![HashMap::new()],
            handlers: Vec::new(),
            inside_handler: Vec::new(),
            olds: Vec::new(),
            entry_states: Vec::new(),
        };

        for item in &module.items {
            match item {
                Item::Function(function) => {
                    if let Some(def) = interp.def_of(&function.sig.name) {
                        interp.functions.insert(def, function);
                    }
                }
                Item::Handler(handler) => {
                    if let Some(def) = interp.def_of(&handler.name) {
                        interp.handler_decls.insert(def, handler);
                    }
                    for field in &handler.state {
                        if let Some(def) = interp.def_of(&field.name) {
                            interp.state_names.insert(def, field.name.name.clone());
                        }
                    }
                }
                Item::Choice(choice) => {
                    for variant in &choice.variants {
                        if let Some(def) = interp.def_of(&variant.name) {
                            interp.variant_names.insert(def, variant.name.name.clone());
                        }
                    }
                }
                Item::TypeAlias(alias) => {
                    if let (Some(def), Some(predicate)) =
                        (interp.def_of(&alias.name), alias.refinement.as_ref())
                    {
                        interp.refinements.insert(def, predicate);
                    }
                }
                _ => {}
            }
        }

        interp
    }

    fn def_of(&self, ident: &Ident) -> Option<DefId> {
        self.resolutions.resolution(ident.span)
    }

    fn kind_of(&self, def: DefId) -> DefKind {
        self.resolutions.def(def).kind
    }

    fn frame(&mut self) -> &mut HashMap<DefId, Value> {
        self.frames.last_mut().expect("there is always a frame")
    }

    // -- failures ----------------------------------------------------------

    fn fail(&self, diagnostic: Diagnostic) -> Signal {
        Signal::Fail(Box::new(diagnostic))
    }

    fn not_runnable(&self, span: Span, what: &str) -> Signal {
        self.fail(
            Diagnostic::error(
                codes::NOT_RUNNABLE,
                self.file,
                span,
                format!("the interpreter cannot run {what} yet"),
            )
            .with_primary_label("not runnable")
            .with_note("this is a gap in the interpreter, not something the language forbids"),
        )
    }

    // -- expressions -------------------------------------------------------

    fn eval(&mut self, expr: &'a Expr) -> Eval<Value> {
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
                if let Some(def) = self.resolutions.resolution(name.span)
                    && self.kind_of(def) == DefKind::Variant
                {
                    return Ok(Value::variant(def, name.name.clone(), Fields::new()));
                }

                let receiver_value = self.eval(receiver)?;
                self.field(&receiver_value, name)
            }

            Expr::Call { callee, args, span } => self.call_expr(callee, args, *span),

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
                    _ => Err(self.not_runnable(*span, "this operator on this value")),
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
                        self.file,
                        *span,
                        format!("no arm of this match accepted {value}"),
                    )
                    .with_primary_label("nothing matched")
                    .with_note("the type checker believes this match is exhaustive, so this is a bug in the interpreter or in the exhaustiveness check"),
                ))
            }

            Expr::Block(block) => self.eval_block(block),

            Expr::Closure { span, .. } => Err(self.not_runnable(*span, "closures")),

            Expr::Old { span, .. } => match self.olds.last().and_then(|olds| olds.get(span)) {
                Some(value) => Ok(value.clone()),
                None => Err(self.not_runnable(*span, "`old` outside a contract")),
            },

            Expr::Unchanged { effect, span } => {
                let Some(def) = self.def_of(&effect.effect) else {
                    return Err(self.not_runnable(*span, "this effect reference"));
                };
                let before = self
                    .entry_states
                    .last()
                    .and_then(|states| states.get(&def))
                    .cloned();
                let now = self.state_of(def);
                Ok(Value::Bool(before == now))
            }

            Expr::With { handlers, body, .. } => {
                let base = self.handlers.len();
                let mut installed = Ok(());
                for handler in handlers {
                    installed = self.install(handler);
                    if installed.is_err() {
                        break;
                    }
                }
                let result = match installed {
                    Ok(()) => self.eval_block(body),
                    Err(signal) => Err(signal),
                };
                self.handlers.truncate(base);
                result
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

    fn read(&mut self, ident: &Ident) -> Eval<Value> {
        let Some(def) = self.def_of(ident) else {
            return Err(self.not_runnable(ident.span, "an unresolved name"));
        };

        if self.kind_of(def) == DefKind::State {
            return self.read_state(def, ident.span);
        }
        if self.kind_of(def) == DefKind::Variant {
            return Ok(Value::variant(def, ident.name.clone(), Fields::new()));
        }

        match self.frames.last().and_then(|frame| frame.get(&def)) {
            Some(value) => Ok(value.clone()),
            None => Err(self.not_runnable(
                ident.span,
                &format!("`{}`, which has no value here", ident.name),
            )),
        }
    }

    fn read_state(&self, def: DefId, span: Span) -> Eval<Value> {
        let Some(index) = self.inside_handler.last().copied() else {
            return Err(self.not_runnable(span, "handler state from outside a handler"));
        };
        let name = &self.state_names[&def];
        match self.handlers[index].state.get(name) {
            Some(value) => Ok(value.clone()),
            None => Err(self.not_runnable(span, "handler state that was never initialised")),
        }
    }

    /// The state of the handler currently installed for `effect`.
    fn state_of(&self, effect: DefId) -> Option<Fields> {
        self.handlers
            .iter()
            .rev()
            .find(|instance| instance.effect == effect)
            .map(|instance| instance.state.clone())
    }

    fn field(&self, value: &Value, name: &Ident) -> Eval<Value> {
        let fields = match value {
            Value::Record(fields) => &**fields,
            Value::Variant(variant) => &variant.fields,
            other => {
                return Err(
                    self.not_runnable(name.span, &format!("field access on {}", other.describe()))
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
                self.file,
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
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };

        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push((self.eval(arg)?, arg.span()));
        }

        let Some(def) = def else {
            return Err(self.not_runnable(callee.span(), "this call"));
        };

        match self.kind_of(def) {
            DefKind::Function => {
                let Some(function) = self.functions.get(&def).copied() else {
                    return Err(self.not_runnable(callee.span(), "this call"));
                };
                self.call(function, values, span, None)
            }
            DefKind::EffectOp => self.dispatch(def, values, span),
            DefKind::Builtin => {
                let name = self.resolutions.def(def).name.clone();
                let carried = values.first().map(|(value, _)| value.clone());
                match (name.as_str(), carried) {
                    ("ok", Some(value)) => Ok(Value::ok(value)),
                    ("err", Some(value)) => Ok(Value::err(value)),
                    _ => Err(self.not_runnable(callee.span(), "this call")),
                }
            }
            DefKind::Import => {
                Err(self.not_runnable(callee.span(), "a call into a module that cannot be loaded"))
            }
            _ => Err(self.not_runnable(callee.span(), "this call")),
        }
    }

    fn dispatch(&mut self, operation: DefId, args: Vec<(Value, Span)>, span: Span) -> Eval<Value> {
        let name = self.resolutions.def(operation).name.clone();
        let Some(effect) = self.resolutions.def(operation).parent else {
            return Err(self.not_runnable(span, "this effect operation"));
        };

        let Some(index) = self
            .handlers
            .iter()
            .rposition(|instance| instance.effect == effect)
        else {
            let effect_name = self.resolutions.def(effect).name.clone();
            return Err(self.fail(
                Diagnostic::error(
                    codes::NO_HANDLER,
                    self.file,
                    span,
                    format!("no handler is installed for `{effect_name}`"),
                )
                .with_primary_label("nothing can perform this")
                .with_note("wrap the call in a `with` block naming a handler for the effect"),
            ));
        };

        let handler_def = self.handlers[index].handler;
        let Some(declaration) = self.handler_decls.get(&handler_def).copied() else {
            return Err(self.not_runnable(span, "this handler"));
        };
        let Some(operation_decl) = declaration
            .operations
            .iter()
            .find(|candidate| candidate.sig.name.name == name)
        else {
            let handler_name = declaration.name.name.clone();
            return Err(self.fail(
                Diagnostic::error(
                    codes::NO_HANDLER,
                    self.file,
                    span,
                    format!("the handler `{handler_name}` does not implement `{name}`"),
                )
                .with_primary_label("not implemented")
                .with_secondary(declaration.name.span, "this handler"),
            ));
        };

        self.call(operation_decl, args, span, Some(index))
    }

    fn call(
        &mut self,
        function: &'a FnDecl,
        args: Vec<(Value, Span)>,
        call_span: Span,
        handler: Option<usize>,
    ) -> Eval<Value> {
        let mut frame = HashMap::new();
        for (param, (value, arg_span)) in function.sig.params.iter().zip(&args) {
            if let Some(def) = self.def_of(&param.name) {
                frame.insert(def, value.clone());
            }
            if let Some(ty) = &param.ty {
                self.check_refinement(ty, value, *arg_span)?;
            }
        }

        self.frames.push(frame);
        if let Some(handler) = handler {
            self.inside_handler.push(handler);
        }

        let result = self.call_body(function, call_span);

        if handler.is_some() {
            self.inside_handler.pop();
        }
        self.frames.pop();
        result
    }

    /// The part of a call that runs inside the new frame.
    fn call_body(&mut self, function: &'a FnDecl, call_span: Span) -> Eval<Value> {
        // Preconditions first. A failure here is the caller's fault, so the
        // diagnostic points at the call and only mentions the clause.
        for requirement in &function.contract.requires {
            if !self.condition(requirement)? {
                let name = function.sig.name.name.clone();
                return Err(self.fail(
                    Diagnostic::error(
                        codes::PRECONDITION_FAILED,
                        self.file,
                        call_span,
                        format!("this call does not satisfy what `{name}` requires"),
                    )
                    .with_primary_label("precondition not met")
                    .with_secondary(requirement.span(), "the clause that failed")
                    .with_note("a precondition failure is a bug in the caller"),
                ));
            }
        }

        self.capture_entry_state(&function.contract.ensures)?;

        let outcome = self.eval_block(&function.body);
        let value = match outcome {
            Ok(value) => value,
            Err(Signal::Return(value)) => value,
            Err(other) => {
                self.olds.pop();
                self.entry_states.pop();
                return Err(other);
            }
        };

        let obligations = self.check_ensures(function, &value, call_span);
        self.olds.pop();
        self.entry_states.pop();
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
            states.insert(instance.effect, instance.state.clone());
        }
        self.entry_states.push(states);

        Ok(())
    }

    fn check_ensures(&mut self, function: &'a FnDecl, value: &Value, call_span: Span) -> Eval<()> {
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
            if !self.condition(&obligation.condition)? {
                let name = function.sig.name.name.clone();
                return Err(self.fail(
                    Diagnostic::error(
                        codes::POSTCONDITION_FAILED,
                        self.file,
                        obligation.span,
                        format!("`{name}` did not keep this promise"),
                    )
                    .with_primary_label("postcondition not met")
                    .with_secondary(call_span, "called from here")
                    .with_note(
                        "a postcondition failure is a bug in the function, not in the caller",
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The `Guarded` tier, actually guarding something.
    fn check_refinement(&mut self, ty: &Type, value: &Value, span: Span) -> Eval<()> {
        let Type::Named { name, .. } = ty else {
            return Ok(());
        };
        let Some(def) = self.def_of(name) else {
            return Ok(());
        };
        let Some(predicate) = self.refinements.get(&def).copied() else {
            return Ok(());
        };

        // The predicate talks about `value`, which is not a name resolution
        // knows about outside the alias, so it is supplied directly.
        let holds = self.eval_predicate(predicate, value)?;
        if holds {
            return Ok(());
        }

        let refinement = name.name.clone();
        Err(self.fail(
            Diagnostic::error(
                codes::REFINEMENT_FAILED,
                self.file,
                span,
                format!("{value} does not satisfy `{refinement}`"),
            )
            .with_primary_label("violates the refinement")
            .with_secondary(predicate.span(), "the predicate it has to satisfy")
            .with_note("the compiler could not prove this statically, so it is checked here"),
        ))
    }

    fn eval_predicate(&mut self, predicate: &'a Expr, value: &Value) -> Eval<bool> {
        match predicate {
            Expr::Ident(ident) if ident.name == "value" => value
                .as_bool()
                .ok_or_else(|| self.not_runnable(ident.span, "a refinement over a non Bool")),
            Expr::Binary {
                op, lhs, rhs, span, ..
            } => {
                let left = self.predicate_operand(lhs, value)?;
                let right = self.predicate_operand(rhs, value)?;
                compare(*op, &left, &right)
                    .ok_or_else(|| self.not_runnable(*span, "this operator inside a refinement"))
            }
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
                ..
            } => Ok(!self.eval_predicate(operand, value)?),
            Expr::Bool { value: literal, .. } => Ok(*literal),
            other => Err(self.not_runnable(other.span(), "this refinement predicate")),
        }
    }

    fn predicate_operand(&mut self, expr: &'a Expr, value: &Value) -> Eval<Value> {
        match expr {
            Expr::Ident(ident) if ident.name == "value" => Ok(value.clone()),
            other => self.eval(other),
        }
    }

    // -- construction ------------------------------------------------------

    fn build(&mut self, path: &'a Expr, fields: &'a [FieldInit], span: Span) -> Eval<Value> {
        let def = match path {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
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
            Some((def, DefKind::Variant)) => {
                let name = self.variant_names[&def].clone();
                Ok(Value::variant(def, name, values))
            }
            _ => Err(self.not_runnable(span, "this literal")),
        }
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
        let Some(def) = def.filter(|def| self.kind_of(*def) == DefKind::Handler) else {
            return Err(self.not_runnable(expr.span(), "this handler"));
        };
        let Some(declaration) = self.handler_decls.get(&def).copied() else {
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
                        self.file,
                        expr.span(),
                        format!("`{handler}` needs an initial value for `{missing}`"),
                    )
                    .with_primary_label("incomplete handler")
                    .with_secondary(field.span, "declared here"),
                ));
            }
        }

        let Some(effect) = self.resolutions.resolution(declaration.effect.span) else {
            return Err(self.not_runnable(expr.span(), "this handler's effect"));
        };

        self.handlers.push(Instance {
            handler: def,
            effect,
            state,
        });
        Ok(())
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

    fn exec(&mut self, stmt: &'a Stmt) -> Eval<()> {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                let value = self.eval(init)?;
                if let Some(ty) = ty {
                    self.check_refinement(ty, &value, init.span())?;
                }
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
                let name = self.state_names[&def].clone();
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

            Stmt::Assert { condition, span } => {
                if self.condition(condition)? {
                    return Ok(());
                }
                Err(self.assertion_failed(condition, *span))
            }

            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(())
            }

            Stmt::Error(span) => Err(self.not_runnable(*span, "code that did not compile")),
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
            self.file,
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
                Some(last) => match self.resolutions.resolution(last.span) {
                    Some(def) if self.kind_of(def) == DefKind::Variant => match value {
                        Value::Variant(variant) => variant.def == def,
                        _ => false,
                    },
                    // A binding matches anything.
                    _ => true,
                },
                None => false,
            },
            Pattern::Record { path, .. } => match (path.last(), value) {
                (Some(last), Value::Variant(variant)) => {
                    self.resolutions.resolution(last.span) == Some(variant.def)
                }
                _ => false,
            },
            Pattern::Tuple { path, .. } => match value {
                Value::Result { ok, .. } => match self.builtin_name(path) {
                    Some(name) => (name == "ok") == *ok,
                    None => false,
                },
                _ => false,
            },
            Pattern::Error(_) => false,
        }
    }

    /// The prelude name a pattern head refers to, if it is one.
    fn builtin_name(&self, path: &[Ident]) -> Option<String> {
        let last = path.last()?;
        let def = self.resolutions.resolution(last.span)?;
        (self.kind_of(def) == DefKind::Builtin).then(|| self.resolutions.def(def).name.clone())
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

fn compare(op: BinaryOp, left: &Value, right: &Value) -> Option<bool> {
    use BinaryOp::*;
    match op {
        Eq => Some(left == right),
        Ne => Some(left != right),
        _ => {
            let (a, b) = (left.as_int()?, right.as_int()?);
            match op {
                Lt => Some(a < b),
                Le => Some(a <= b),
                Gt => Some(a > b),
                Ge => Some(a >= b),
                _ => None,
            }
        }
    }
}
