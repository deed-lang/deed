//! The syntax tree.
//!
//! This layer answers "is this a well formed program" and nothing else. It does
//! not know what a name refers to, whether a type exists, or whether an effect
//! is declared. Those are later questions and mixing them in here is how a
//! parser turns into a compiler nobody can follow.
//!
//! Two conventions run through the whole tree.
//!
//! **Every node carries a span.** Diagnostics are only as good as the source
//! range they can point at, and a node that cannot be located is a node that
//! can only produce a vague error.
//!
//! **Errors are nodes, not absences.** [`Expr::Error`], [`Type::Error`] and
//! [`Pattern::Error`] mean the parser gave up on a subtree but kept its extent.
//! Later passes skip them instead of tripping over a missing branch, which is
//! what keeps one syntax error from silencing everything after it.
//!
//! An item and a statement have no such node, because they do not need one.
//! Both live in a list, and a list with one entry dropped is still the shape
//! every pass expects. The rule is about positions where an absence would
//! change what the tree means.

use deed_diagnostics::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

/// A module path such as `payments/transfer`.
#[derive(Clone, Debug)]
pub struct ModulePath {
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl ModulePath {
    pub fn to_string_path(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// `use std/result.{Result, ok, err}`
///
/// There is no wildcard form. If a name is in scope, some line in the file put
/// it there, which is P1 applied to imports.
#[derive(Clone, Debug)]
pub struct Use {
    pub path: ModulePath,
    pub names: Vec<Ident>,
    pub span: Span,
}

/// An edition declaration on a module.
#[derive(Clone, Debug)]
pub struct EditionDecl {
    pub year: u32,
    pub span: Span,
}

/// One file.
#[derive(Clone, Debug)]
pub struct Module {
    pub name: Option<ModulePath>,
    pub edition: Option<EditionDecl>,
    pub uses: Vec<Use>,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Deprecate(DeprecateDecl),
    Operator(OperatorDecl),
    TypeAlias(TypeAlias),
    Record(RecordDecl),
    Choice(ChoiceDecl),
    Effect(EffectDecl),
    Handler(HandlerDecl),
    Function(FnDecl),
    Test(TestDecl),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Deprecate(d) => d.span,
            Item::Operator(d) => d.span,
            Item::TypeAlias(d) => d.span,
            Item::Record(d) => d.span,
            Item::Choice(d) => d.span,
            Item::Effect(d) => d.span,
            Item::Handler(d) => d.span,
            Item::Function(d) => d.span,
            Item::Test(d) => d.span,
        }
    }
}

/// `operator + = added`
///
/// Says that an operator, written between two values of a type this module
/// declares, means a function this module declares. A binding rather than a
/// definition: the function keeps its name, so it can still be called and
/// still be passed. See
/// `design/decisions/2026-08-03-operators-bound-to-functions.md`.
#[derive(Clone, Debug)]
pub struct OperatorDecl {
    pub op: BinaryOp,
    /// Where the operator itself is written, which is what a diagnostic about
    /// the choice of operator underlines.
    pub op_span: Span,
    pub function: Ident,
    pub span: Span,
}

/// `deprecated old_name -> new_name`
///
/// A declaration-level migration marker. `old_name` remains available for now,
/// but every use warns and points at `new_name`.
#[derive(Clone, Debug)]
pub struct DeprecateDecl {
    pub old: Ident,
    pub new: Ident,
    pub span: Span,
}

/// `type Positive = Int where value > 0`
///
/// The refinement is what stops most validation from needing to exist. A
/// `Positive` cannot be constructed without the predicate holding, so nothing
/// downstream re-checks it.
#[derive(Clone, Debug)]
pub struct TypeAlias {
    pub name: Ident,
    /// `<K, V>` in `type Table<K, V> = List<Entry<K, V>>`.
    ///
    /// Only on an alias with no refinement. A predicate over a value whose
    /// type is not known yet is a different question, and `DEED4028` says so
    /// rather than guessing an answer to it.
    pub generics: Vec<Ident>,
    pub ty: Type,
    pub refinement: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: Ident,
    pub ty: Type,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RecordDecl {
    pub name: Ident,
    /// `<A, B>` in `record Pair<A, B> { left: A, right: B }`.
    pub generics: Vec<Ident>,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: Ident,
    /// `None` for a variant with no payload, such as `LimitExceeded`.
    pub fields: Option<Vec<FieldDecl>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ChoiceDecl {
    pub name: Ident,
    /// `<T>` in `choice Option<T> { None, Some { value: T } }`.
    pub generics: Vec<Ident>,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EffectDecl {
    pub name: Ident,
    /// `<uses r>` in `effect Task<uses r>`.
    ///
    /// A row variable an effect declares stands for whatever the values handed
    /// to its operations perform. It is what lets a handler keep a queue of
    /// them without naming, in the library, effects only the program knows
    /// about.
    pub rows: Vec<Ident>,
    /// `from "wasi:random/random"`, when the effect names where its
    /// operations come from.
    ///
    /// An effect nothing handles reaches the program's boundary, and at that
    /// boundary it is an import: a compiled component asks its host for it.
    /// Without this the import can only be named `deed:<effect>`, which is a
    /// fine default for an interface this program invented and useless for one
    /// that already exists somewhere else. Writing it down is what lets a
    /// program ask for an interface it did not define.
    ///
    /// It says nothing about whether the effect is handled. A handler
    /// discharges it and it leaves the row, and then there is no import,
    /// which is the same rule every other effect follows.
    pub interface: Option<Interface>,
    /// Operation signatures. An effect has no bodies; that is the point of it.
    pub operations: Vec<FnSig>,
    pub span: Span,
}

/// A world-level name for an effect's operations.
#[derive(Clone, Debug)]
pub struct Interface {
    /// The text between the quotes, unvalidated here. What counts as a WIT
    /// interface name is the checker's question, not the parser's.
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct HandlerDecl {
    pub name: Ident,
    pub effect: Ident,
    pub state: Vec<FieldDecl>,
    pub operations: Vec<FnDecl>,
    /// Cleanup block that runs whenever the `with` block that installed this
    /// handler exits, whether normally or because a contract failed.
    pub finally: Option<Block>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TestDecl {
    pub name: String,
    pub name_span: Span,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    /// Optional because handler operations inherit their types from the effect
    /// they implement.
    pub ty: Option<Type>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub name: Ident,
    /// `<T>` in `fn first<T>(items: List<T>) -> Result<T, String>`.
    ///
    /// Every one of these has to appear in a parameter's type, which is what
    /// removes the need to write type arguments at a call site and with them
    /// the `f<a>(b)` versus `f < a > (b)` ambiguity. See `design/02-syntax.md`.
    pub generics: Vec<Ident>,
    /// `uses r` in `fn map<A, B, uses r>(..)`, written in the same list.
    ///
    /// A row variable stands for whatever a callback performs, so that a
    /// combinator can pass it through to its own row rather than naming one
    /// effect and being useful for that effect only. Declared rather than
    /// inferred from where it appears, because a name that means one thing in
    /// one position and another thing elsewhere is a thing a reader has to
    /// work out.
    pub rows: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub sig: FnSig,
    pub contract: Contract,
    pub body: Block,
    pub span: Span,
}

/// The contract block: everything between the return type and the opening brace.
///
/// This is the review surface. A person reads a [`FnSig`] and a `Contract` and
/// is entitled to stop there.
#[derive(Clone, Debug, Default)]
pub struct Contract {
    /// `where`, what the caller must guarantee.
    pub requires: Vec<Expr>,
    /// `uses`, every effect the body may perform. Empty means pure.
    pub uses: Vec<EffectRef>,
    /// `ensures`, what the function guarantees, per outcome.
    pub ensures: Vec<Ensures>,
    pub span: Option<Span>,
}

impl Contract {
    pub fn is_empty(&self) -> bool {
        self.requires.is_empty() && self.uses.is_empty() && self.ensures.is_empty()
    }

    /// A function that declares no effects is pure.
    pub fn is_pure(&self) -> bool {
        self.uses.is_empty()
    }
}

/// `Ledger`, `Ledger.read`, or `sys.*`.
#[derive(Clone, Debug)]
pub struct EffectRef {
    pub effect: Ident,
    /// `None` means the whole effect, so every operation it declares.
    pub operation: Option<Ident>,
    /// True for the `sys.*` form.
    pub all: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Err,
}

/// One `ensures` obligation, such as `err => unchanged(Ledger)`.
///
/// Obligations are stated per outcome so that neither the success case nor the
/// failure case can be left unsaid by accident.
#[derive(Clone, Debug)]
pub struct Ensures {
    pub outcome: Outcome,
    pub outcome_span: Span,
    pub condition: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Type {
    /// `Money`, `Result<Receipt, TransferError>`
    Named {
        name: Ident,
        args: Vec<Type>,
        span: Span,
    },
    /// `Fn(Int, Int) -> Int`, or `Fn(String) uses Io.write -> ()`
    ///
    /// The row goes before the arrow, which is not where a contract's `uses`
    /// goes, and that is the reason. A function declaration's contract also
    /// starts with `uses` and also comes after a return type, so
    /// `fn f() -> Fn(Int) -> Int uses Log.note` would have two readings and no
    /// way to tell them apart. Before the arrow there is nothing to confuse it
    /// with: the `->` ends the list.
    ///
    /// No row means the function performs nothing. Leaving one off cannot mean
    /// "any row": a value carrying an unstated effect through a signature
    /// would undo the whole point of having rows.
    Fn {
        params: Vec<Type>,
        row: Vec<EffectRef>,
        ret: Box<Type>,
        span: Span,
    },
    Unit(Span),
    Error(Span),
}

impl Type {
    pub fn span(&self) -> Span {
        match self {
            Type::Named { span, .. }
            | Type::Fn { span, .. }
            | Type::Unit(span)
            | Type::Error(span) => *span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// A trailing expression with no terminator, which is the block's value.
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        pattern: Pattern,
        ty: Option<Type>,
        init: Expr,
        span: Span,
    },
    /// `count = count + by`
    ///
    /// The target must be a `state` field of the enclosing handler. State is
    /// the only mutable thing in the language, which is what lets an empty
    /// effect row mean that a function cannot observe or cause a change to
    /// anything.
    Assign {
        target: Ident,
        value: Expr,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Assert {
        condition: Expr,
        span: Span,
    },
    /// `assert refuses order_of(0)`
    ///
    /// The claim that evaluating this breaks a contract. A precondition, a
    /// postcondition or a refinement, and nothing else: overflow and a missing
    /// handler are not contracts and are not what this is for.
    ///
    /// It exists because a test could not reach the `Guarded` tier at all. A
    /// contract failure ends the run, so a file of examples that tried to show
    /// one could not pass, and once preconditions were checked at the call
    /// site the checker refused the file outright.
    Refuses {
        subject: Expr,
        span: Span,
    },
    /// `abandon`
    ///
    /// Unwinds the current computation unconditionally. Used inside a handler
    /// operation to signal that the computation which performed the effect
    /// should not receive a value back; instead the stack unwinds, running
    /// cleanup blocks for installed handlers.
    ///
    /// The abandoned computation observes `DEED6011`. `assert refuses` cannot
    /// catch it, because it is not a contract failure.
    Abandon {
        span: Span,
    },
    Expr(Expr),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Refuses { span, .. }
            | Stmt::Assert { span, .. }
            | Stmt::Abandon { span } => *span,
            Stmt::Expr(expr) => expr.span(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinaryOp {
    /// Every binary operator the language has.
    ///
    /// Written out, because there is no way to ask an enum for its variants,
    /// and something that wants to measure the operators has to be able to
    /// walk them. `crates/deed-driver/tests/documentation.rs` does exactly
    /// that: it asks the type checker which types each of these takes, and
    /// holds the sentence in the corpus that says which ones mean more than
    /// one thing. An entry here that cannot be written fails that test by
    /// name, because a probe over it will not compile for any type.
    pub const ALL: [BinaryOp; 13] = [
        BinaryOp::Or,
        BinaryOp::And,
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
        BinaryOp::Add,
        BinaryOp::Sub,
        BinaryOp::Mul,
        BinaryOp::Div,
        BinaryOp::Rem,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Or => "||",
            BinaryOp::And => "&&",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
        }
    }

    /// The operators a module may bind to one of its own functions.
    ///
    /// The three total arithmetic ones, and `<`. `/` and `%` are partial, and
    /// this language spells a partial answer with `Result`, which is a shape
    /// an operator cannot have without `a / b + c` meaning something nobody
    /// expects. `==` is total and structural over every type already.
    ///
    /// `<` is here and `<=`, `>` and `>=` are not, because binding four
    /// separately lets them disagree: `a < b` and `b > a` could answer
    /// differently and nothing would say so. One binding answers all four, by
    /// swapping the operands and negating, which is what an order is. See
    /// `design/decisions/2026-08-04-one-binding-for-an-order.md`.
    pub const BINDABLE: [BinaryOp; 4] = [BinaryOp::Lt, BinaryOp::Add, BinaryOp::Sub, BinaryOp::Mul];

    pub fn is_bindable(self) -> bool {
        matches!(
            self,
            BinaryOp::Lt | BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
        )
    }

    /// How a comparison is answered by a binding for `<`.
    ///
    /// `Some((swap, negate))`: hand the operands to the bound function in that
    /// order, and negate what comes back. `None` for an operator no order
    /// answers.
    pub fn from_less_than(self) -> Option<(bool, bool)> {
        match self {
            BinaryOp::Lt => Some((false, false)),
            BinaryOp::Gt => Some((true, false)),
            BinaryOp::Ge => Some((false, true)),
            BinaryOp::Le => Some((true, true)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// A field in a struct literal. `value` is `None` for the shorthand form,
/// where `Receipt { from }` means `from: from`.
#[derive(Clone, Debug)]
pub struct FieldInit {
    pub name: Ident,
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

/// `with sum = 0`, the accumulator of a `for`.
#[derive(Clone, Debug)]
pub struct Accumulator {
    pub name: Ident,
    pub init: Box<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int {
        value: i64,
        span: Span,
    },
    Str {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    Unit(Span),
    Ident(Ident),

    /// `a.b`
    ///
    /// Module qualification and field access look identical at this stage, and
    /// the parser has no way to tell them apart. Both produce this node and
    /// name resolution decides. Inventing two node kinds here would only push
    /// a guess earlier than the information arrives.
    Field {
        receiver: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `[1, 2, 3]`
    ///
    /// The only way to write a list down. There is no constructor call for
    /// one, because a literal is the form every reader already knows and a
    /// second spelling would be a second thing to learn for no gain.
    List {
        elements: Vec<Expr>,
        span: Span,
    },
    StructLit {
        path: Box<Expr>,
        fields: Vec<FieldInit>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        op_span: Span,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        op_span: Span,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    /// `expr?`, which propagates the error case.
    Try {
        operand: Box<Expr>,
        span: Span,
    },
    If {
        condition: Box<Expr>,
        then_branch: Block,
        else_branch: Option<Box<Expr>>,
        span: Span,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// `for n in numbers with sum = 0 { sum + n }`
    ///
    /// A fold with syntax, and not a loop with a variable in it. `sum` is bound
    /// again on each turn rather than assigned, so nothing here is mutable and
    /// the claim that a handler's `state` is the only mutable thing in the
    /// language survives having iteration at all.
    ///
    /// The block's value is the accumulator for the next turn, and the value of
    /// the whole expression is the last one. Leaving `with` off means an
    /// accumulator of `()`, which is the loop that exists for its effects.
    For {
        /// The element, bound once per turn.
        binder: Ident,
        /// Where in the list that element was, bound once per turn as well.
        ///
        /// `None` for a walk that does not care, which is most of them. A
        /// `for` that never says where it is means every walk that does care
        /// has to carry a counter in a record and remember to bump it in every
        /// branch, which is three walks in `examples/todo.deed` alone.
        index: Option<Ident>,
        iterable: Box<Expr>,
        /// The accumulator and what it starts as, when there is one.
        accumulator: Option<Accumulator>,
        /// `while so_far` in `for x in xs with so_far = true while so_far`.
        ///
        /// Read before each turn, with the accumulator in scope and the
        /// element not, since the element belongs to the turn this is deciding
        /// whether to take. Stopping early rather than looping longer: the
        /// list still bounds how many turns there can be, so this cannot bring
        /// back the termination problem that keeps `while` out of the language
        /// as a statement.
        ///
        /// Needs an accumulator. The condition is about what the walk has
        /// worked out so far, and a condition that can only read things the
        /// walk never changes either stops it before it starts or never stops
        /// it at all.
        keep: Option<Box<Expr>>,
        body: Block,
        span: Span,
    },
    Block(Block),
    Closure {
        params: Vec<Param>,
        body: Box<Expr>,
        span: Span,
    },

    /// `old(expr)`, the value of `expr` in the state on entry.
    ///
    /// A keyword rather than a call, because no function can reach a previous
    /// state, and the tree should not pretend otherwise.
    Old {
        expr: Box<Expr>,
        span: Span,
    },
    /// `unchanged(Ledger)`, which takes an effect rather than a value.
    Unchanged {
        effect: EffectRef,
        span: Span,
    },
    /// `with SomeHandler, Another { ... }`
    With {
        handlers: Vec<Expr>,
        body: Block,
        span: Span,
    },

    Error(Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int { span, .. }
            | Expr::Str { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Unit(span)
            | Expr::Field { span, .. }
            | Expr::Call { span, .. }
            | Expr::List { span, .. }
            | Expr::StructLit { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Try { span, .. }
            | Expr::If { span, .. }
            | Expr::Match { span, .. }
            | Expr::For { span, .. }
            | Expr::Closure { span, .. }
            | Expr::Old { span, .. }
            | Expr::Unchanged { span, .. }
            | Expr::With { span, .. }
            | Expr::Error(span) => *span,
            Expr::Ident(ident) => ident.span,
            Expr::Block(block) => block.span,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Expr::Error(_))
    }

    /// `Int.max` and `Int.min`, as the number they stand for.
    ///
    /// `Int` is a signed 64-bit integer and a program has to be able to say
    /// so. It reads as a field access because that is what it looks like, and
    /// every pass that has to know asks here rather than matching the shape
    /// again: the checker for the type and the range, the interpreter and the
    /// backend for the value.
    ///
    /// Shape alone. A file that declares something called `Int` is warned that
    /// it hid a builtin, and the checker asks whether the member resolved
    /// before it asks this; everything downstream of the checker is looking at
    /// a program that already type checked.
    pub fn int_limit(&self) -> Option<i64> {
        let Expr::Field { receiver, name, .. } = self else {
            return None;
        };
        let Expr::Ident(ident) = &**receiver else {
            return None;
        };
        if ident.name != "Int" {
            return None;
        }
        match name.name.as_str() {
            "max" => Some(i64::MAX),
            "min" => Some(i64::MIN),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PatternField {
    pub name: Ident,
    /// `None` for the shorthand form, where `{ available }` binds `available`.
    pub pattern: Option<Pattern>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Wildcard(Span),
    /// A dotted name.
    ///
    /// A single segment is a binding unless it resolves to a variant with no
    /// payload. The parser cannot know which, so it does not guess.
    Path {
        segments: Vec<Ident>,
        span: Span,
    },
    /// `Ok(receipt)`
    Tuple {
        path: Vec<Ident>,
        elements: Vec<Pattern>,
        span: Span,
    },
    /// `InsufficientFunds { available }`
    Record {
        path: Vec<Ident>,
        fields: Vec<PatternField>,
        span: Span,
    },
    Int {
        value: i64,
        span: Span,
    },
    Str {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    /// `Plus | Times | Close`, in a match arm and nowhere else.
    ///
    /// Every alternative is written out, so adding a variant to the choice
    /// still breaks every match that has to care, which is the whole of what
    /// the no catch-all rule asks for. What it does not ask for is repeating
    /// the body once per variant.
    ///
    /// No alternative binds anything. That is what keeps this small: a
    /// language where alternatives can bind has to require that all of them
    /// bind the same names, and the question does not come up if none of them
    /// binds at all. A variant with fields is matched by name alone here, the
    /// same way it can be anywhere else.
    OneOf {
        alternatives: Vec<Pattern>,
        span: Span,
    },
    Error(Span),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(span)
            | Pattern::Path { span, .. }
            | Pattern::Tuple { span, .. }
            | Pattern::Record { span, .. }
            | Pattern::Int { span, .. }
            | Pattern::Str { span, .. }
            | Pattern::Bool { span, .. }
            | Pattern::OneOf { span, .. }
            | Pattern::Error(span) => *span,
        }
    }
}
