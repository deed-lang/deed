//! A mid-level IR for the native backend, lowered from the type- and
//! effect-checked tree `deed-typeck` and `deed-effects` already produce.
//!
//! See `design/05-backend.md` for why this exists and `crates/deed-codegen`
//! for what reads it.
//!
//! Everything here is explicit in a way the surface syntax is not: types are
//! resolved rather than inferred, locals are numbered rather than named, and
//! nothing needs a symbol table to read. That is the point of the layer. A
//! backend that had to resolve a name would be a second checker, and the two
//! of them disagreeing is the bug this split rules out.

pub mod lower;

pub use lower::{Unlowered, lower};

/// The diagnostic codes a compiled program can stop with.
///
/// The `DEED6xxx` range belongs to `deed-interp`, which owns the vocabulary
/// and documents what each code covers. These are the ones a compiled
/// program can reach, spelled here rather than depended on because the
/// dependency would run the wrong way: a backend that needed the interpreter
/// to build is a backend that could not replace it.
///
/// `crates/deed-driver/tests/failures.rs` pins them to the interpreter's, so
/// the copy cannot drift without something saying so.
pub mod codes {
    /// An `assert` that was not true.
    pub const ASSERTION_FAILED: &str = "DEED6001";
    /// A `where` clause that did not hold on entry.
    pub const PRECONDITION_FAILED: &str = "DEED6002";
    /// Something the run could not do, which is where a match running out of
    /// arms lands.
    pub const NOT_RUNNABLE: &str = "DEED6006";
}

/// The types a value can have at this level.
///
/// Deliberately smaller than `deed_typeck::Ty`. Refinements are gone: a
/// refinement is a claim about a value that the checker either proved or
/// turned into a runtime check, and either way what is left to compile is
/// the base type. Type parameters are gone too, because a generic function
/// reaches this layer once per set of type arguments it is called with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    Unit,
    Bool,
    Int,
    Str,
    /// A list of one element type, boxed because a list of lists is a list.
    List(Box<Ty>),
    /// A record or a choice, by the index of its layout in [`Program::layouts`].
    Aggregate(LayoutId),
    /// An opaque capability handle. Nothing in compiled code may look inside
    /// one, which is the whole of what a capability is.
    ///
    /// Not boxed: a handle is a number the host gave out and the program
    /// hands back, and it names nothing in the program's own memory. That is
    /// what makes it opaque rather than merely undocumented. A compiled
    /// program cannot forge one, because the only things it can do with a
    /// capability are pass it on and hand it to the host.
    Capability,
    /// A function value: a code pointer and a captured environment.
    Closure,
}

impl Ty {
    /// Whether a value of this type is a reference to something rather than
    /// a number that fits in a machine word.
    ///
    /// A property of the type rather than of any one use of it, which is why
    /// it is answered here and not at each load and store.
    pub fn is_boxed(&self) -> bool {
        matches!(self, Ty::Str | Ty::List(_) | Ty::Aggregate(_) | Ty::Closure)
    }
}

/// Which record or choice layout, by position in [`Program::layouts`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LayoutId(pub usize);

/// Which function, by position in [`Program::functions`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FuncId(pub usize);

/// Which slot within one function body.
///
/// Parameters come first, in order, so slot `i` of a function taking `n`
/// parameters is a parameter exactly when `i < n`. Nothing else needs to be
/// recorded about where a slot came from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Local(pub usize);

/// How a record or a choice is laid out.
///
/// One shape for both, because a record is a choice with a single variant
/// and carrying that distinction this far would mean two of everything
/// below.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Layout {
    pub name: String,
    /// One entry per variant. A record has exactly one.
    pub variants: Vec<Variant>,
}

impl Layout {
    /// Whether reading this has to look at a discriminant first.
    ///
    /// A record does not have one. There is nothing to tell apart, and a tag
    /// that is always zero would cost a word on every aggregate in the
    /// language.
    pub fn is_tagged(&self) -> bool {
        self.variants.len() > 1
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EffectId(pub usize);

/// An effect, reduced to what dispatch needs.
///
/// The operations are in declaration order and a `Perform` names one by
/// position, so nothing at this level does a lookup by string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Effect {
    pub name: String,
    pub operations: Vec<String>,
}

/// A whole program, ready to compile.
///
/// Self-contained on purpose: nothing here points back at a syntax tree, a
/// `SourceMap` or a resolution table. What a backend needs is in here, or it
/// was not lowered.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Program {
    pub layouts: Vec<Layout>,
    pub effects: Vec<Effect>,
    pub functions: Vec<Function>,
    /// Which function `deed run` calls, when the program has one.
    pub entry: Option<FuncId>,
}

impl Program {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_layout(&mut self, layout: Layout) -> LayoutId {
        self.layouts.push(layout);
        LayoutId(self.layouts.len() - 1)
    }

    pub fn add_function(&mut self, function: Function) -> FuncId {
        self.functions.push(function);
        FuncId(self.functions.len() - 1)
    }

    pub fn add_effect(&mut self, effect: Effect) -> EffectId {
        self.effects.push(effect);
        EffectId(self.effects.len() - 1)
    }

    pub fn layout(&self, id: LayoutId) -> &Layout {
        &self.layouts[id.0]
    }

    pub fn effect(&self, id: EffectId) -> &Effect {
        &self.effects[id.0]
    }

    pub fn function(&self, id: FuncId) -> &Function {
        &self.functions[id.0]
    }

    /// The function with this name, if there is one.
    ///
    /// Names are unique here. A generic function is lowered once per set of
    /// type arguments and each copy is named for the arguments it was
    /// lowered with, so two functions sharing a name is a bug in lowering
    /// rather than something to resolve at this level.
    pub fn find(&self, name: &str) -> Option<FuncId> {
        self.functions
            .iter()
            .position(|function| function.name == name)
            .map(FuncId)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    /// Unique within a program, and the symbol the compiled body gets.
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// The type of every slot, parameters first.
    pub locals: Vec<Ty>,
    pub body: Block,
}

impl Function {
    /// A function with parameters and nothing in it yet.
    pub fn new(name: impl Into<String>, params: Vec<Ty>, ret: Ty) -> Self {
        Function {
            name: name.into(),
            locals: params.clone(),
            params,
            ret,
            body: Block::of(Expr::Unit),
        }
    }

    /// Adds a slot and hands back its number.
    pub fn add_local(&mut self, ty: Ty) -> Local {
        self.locals.push(ty);
        Local(self.locals.len() - 1)
    }

    pub fn local_ty(&self, local: Local) -> &Ty {
        &self.locals[local.0]
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stmt {
    /// Bind a slot to a value.
    Assign { local: Local, value: Expr },
    /// Evaluate something for what it does rather than what it produces.
    Discard(Expr),
    /// Stop with a contract failure.
    ///
    /// Only reached from an obligation the checker left `Guarded`. A `Proven`
    /// one emits nothing at all, which is the whole of what the tier buys at
    /// runtime.
    ///
    /// Carries the diagnostic code as well as the sentence, because a run
    /// that stops should say the same thing whichever engine ran it, and the
    /// code is what makes two messages comparable.
    Fail { code: String, message: String },
    /// Run the body while the condition holds, checking it first.
    ///
    /// A `for` in Deed is a fold over a list rather than a loop, and this is
    /// what it becomes: a counter, a bound, and a body that rebinds the
    /// accumulator. The language has no `while` statement and this is not
    /// one; nothing lowers to it except a walk whose turns a list already
    /// bounds.
    While { condition: Expr, body: Vec<Stmt> },
    /// Write one field of an aggregate in place.
    ///
    /// The only thing in this IR that changes something already built, and
    /// it exists for the only thing in the language that can: a handler's
    /// `state`. Nothing else lowers to it. A record is built once by `Make`
    /// and read by `Field` from then on.
    SetField {
        object: Expr,
        layout: LayoutId,
        variant: usize,
        field: usize,
        value: Expr,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Expr {
    Unit,
    Bool(bool),
    Int(i64),
    Str(String),
    /// Read a slot.
    Local(Local),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// A direct call to a known function.
    Call {
        func: FuncId,
        args: Vec<Expr>,
    },
    /// A call through a function value, which carries its environment
    /// alongside the code pointer.
    CallIndirect {
        callee: Box<Expr>,
        args: Vec<Expr>,
        /// What comes back, since a code pointer does not say.
        ret: Box<Ty>,
    },
    /// Build a record, or one variant of a choice.
    Make {
        layout: LayoutId,
        variant: usize,
        fields: Vec<Expr>,
    },
    /// Read one field of an aggregate.
    Field {
        value: Box<Expr>,
        layout: LayoutId,
        variant: usize,
        field: usize,
    },
    /// Which variant an aggregate holds, as a number.
    Discriminant {
        value: Box<Expr>,
        layout: LayoutId,
    },
    List {
        element: Box<Ty>,
        items: Vec<Expr>,
    },
    /// Both arms produce a value, because `if` is an expression in Deed.
    /// Flattening it into statements here would need a slot per branch and a
    /// join that nothing else in this IR has.
    If {
        condition: Box<Expr>,
        then: Box<Block>,
        otherwise: Box<Block>,
        ty: Box<Ty>,
    },
    /// Statements, then a value.
    Block(Box<Block>),
    /// A call into the runtime support library.
    ///
    /// The names are a closed set that [`runtime`] publishes. A backend does
    /// not need to know what any of them do, only how to call one.
    Runtime {
        name: &'static str,
        args: Vec<Expr>,
        ret: Box<Ty>,
    },
    /// One element of a list, by position, with nothing checking the bound.
    ///
    /// Total on purpose, and only produced where the bound is already known:
    /// a walk generates its own index from the list's own length. The
    /// prelude's `at`, which anybody can call with anything, is a different
    /// thing and still hands back a `Result`.
    ElementAt {
        list: Box<Expr>,
        index: Box<Expr>,
        element: Box<Ty>,
    },
    /// Run a block with a handler answering for an effect.
    ///
    /// The handler is in scope for the block and no longer once it ends,
    /// which is what makes this an expression that wraps a body rather than
    /// a statement that installs something. Nesting is what decides which
    /// handler answers: the innermost one that names the effect.
    Install {
        effect: EffectId,
        /// What the handler's `state` starts as, or `Unit` when it declares
        /// none. One cell per installation, not one per handler declaration,
        /// so two `with` blocks over the same handler do not share it.
        state: Box<Expr>,
        /// One function per operation the effect declares, in that order.
        /// Each takes the state cell first and its own parameters after.
        operations: Vec<FuncId>,
        body: Box<Block>,
        ty: Box<Ty>,
    },
    /// Perform an operation, answered by whichever handler is innermost.
    ///
    /// Which one that is cannot be read off the call site, because the
    /// function performing may have been called from inside any number of
    /// `with` blocks and is compiled once. So this is a search at runtime,
    /// and the thing it searches is the only piece of state the compiled
    /// program keeps that the source does not name.
    ///
    /// It is a search and a call, and nothing else. An operation runs once
    /// per `perform` and its answer is a return value, so there is no
    /// continuation to capture and nothing to resume. See `design/05-backend.md`.
    Perform {
        effect: EffectId,
        operation: usize,
        args: Vec<Expr>,
        ret: Box<Ty>,
    },
    /// A call to something the host supplies rather than the program.
    ///
    /// Everything a Deed program can do that is not arithmetic on its own
    /// memory is one of these: reading a file, writing a line, asking the
    /// clock, narrowing a capability. None of them are things a compiled
    /// module can do by itself, and that is the point rather than a
    /// limitation. The module says what it wants and whoever runs it
    /// decides, which is the same shape the language's capabilities already
    /// have.
    ///
    /// The name is `namespace.operation`, and it becomes a WebAssembly
    /// import. A call still needs the capability, which is the first
    /// argument: the effect row says what kind of thing is happening and the
    /// handle says which resource it happens to, and neither is enough
    /// alone. See `crates/deed-typeck/src/check.rs` for where that rule is
    /// stated and `design/04-capabilities.md` for why.
    Host {
        name: String,
        args: Vec<Expr>,
        ret: Box<Ty>,
    },
}

/// Statements, then the value the whole thing has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub value: Expr,
}

impl Block {
    pub fn of(value: Expr) -> Self {
        Block {
            stmts: Vec::new(),
            value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    Not,
    Negate,
}

/// The operators, already resolved to one meaning each.
///
/// `+` on two strings and `+` on two numbers are separate entries here.
/// design/02-syntax.md counts five operators in the surface language that
/// mean more than one thing; all five are split by the time they reach this
/// layer, so nothing downstream has to ask what the operands were.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinaryOp {
    AddInt,
    SubInt,
    MulInt,
    DivInt,
    RemInt,
    ConcatStr,
    /// Structural, and total: two values of the same type always compare.
    Eq,
    Ne,
    LtInt,
    LeInt,
    GtInt,
    GeInt,
    LtStr,
    LeStr,
    GtStr,
    GeStr,
    And,
    Or,
}

impl BinaryOp {
    pub fn result_ty(&self) -> Ty {
        match self {
            BinaryOp::AddInt
            | BinaryOp::SubInt
            | BinaryOp::MulInt
            | BinaryOp::DivInt
            | BinaryOp::RemInt => Ty::Int,
            BinaryOp::ConcatStr => Ty::Str,
            _ => Ty::Bool,
        }
    }
}

/// What the runtime library publishes.
///
/// Written out rather than spelled at each call site, so a typo is a compile
/// error in the compiler rather than a missing symbol in somebody's program.
pub mod runtime {
    pub const STR_CONCAT: &str = "deed_rt_str_concat";
    pub const STR_EQ: &str = "deed_rt_str_eq";
    pub const STR_CMP: &str = "deed_rt_str_cmp";
    pub const STR_LEN: &str = "deed_rt_str_len";
    pub const LIST_NEW: &str = "deed_rt_list_new";
    pub const LIST_PUSH: &str = "deed_rt_list_push";
    pub const LIST_AT: &str = "deed_rt_list_at";
    pub const LIST_LEN: &str = "deed_rt_list_len";
    pub const CONTRACT_FAILED: &str = "deed_rt_contract_failed";

    /// Every name above, so something can check a call against the set.
    pub const ALL: &[&str] = &[
        STR_CONCAT,
        STR_EQ,
        STR_CMP,
        STR_LEN,
        LIST_NEW,
        LIST_PUSH,
        LIST_AT,
        LIST_LEN,
        CONTRACT_FAILED,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(name: &str) -> Effect {
        Effect {
            name: name.to_string(),
            operations: Vec::new(),
        }
    }

    fn layout(name: &str) -> Layout {
        Layout {
            name: name.to_string(),
            variants: Vec::new(),
        }
    }

    /// Adding something hands back the identifier that reads it back.
    ///
    /// Worth asking directly rather than through a lowered program, because
    /// a program that numbered everything consistently wrong would still
    /// run: an installation and a `perform` comparing the same wrong number
    /// agree. What that would break is anybody reading the MIR, and this is
    /// what would say so.
    #[test]
    fn what_goes_in_comes_back_out_under_the_identifier_it_was_given() {
        let mut program = Program::new();

        let counter = program.add_effect(effect("Counter"));
        let clock = program.add_effect(effect("Clock"));
        assert_eq!(program.effect(counter).name, "Counter");
        assert_eq!(program.effect(clock).name, "Clock");
        assert_ne!(counter, clock);

        let pair = program.add_layout(layout("Pair"));
        let tone = program.add_layout(layout("Tone"));
        assert_eq!(program.layout(pair).name, "Pair");
        assert_eq!(program.layout(tone).name, "Tone");

        let first = program.add_function(Function::new("first", Vec::new(), Ty::Unit));
        let second = program.add_function(Function::new("second", Vec::new(), Ty::Unit));
        assert_eq!(program.function(first).name, "first");
        assert_eq!(program.function(second).name, "second");
        assert_eq!(program.find("second"), Some(second));
        assert_eq!(program.find("third"), None);
    }

    /// A choice has something to tell apart and a record does not.
    #[test]
    fn only_a_layout_with_more_than_one_variant_carries_a_tag() {
        let variant = |name: &str| Variant {
            name: name.to_string(),
            fields: Vec::new(),
        };

        assert!(
            !Layout {
                name: "Pair".to_string(),
                variants: vec![variant("Pair")],
            }
            .is_tagged()
        );
        assert!(
            Layout {
                name: "Tone".to_string(),
                variants: vec![variant("Plain"), variant("Loud")],
            }
            .is_tagged()
        );
    }

    /// What lives in memory and what fits in a slot.
    #[test]
    fn a_type_says_whether_it_lives_in_memory() {
        assert!(!Ty::Unit.is_boxed());
        assert!(!Ty::Bool.is_boxed());
        assert!(!Ty::Int.is_boxed());
        assert!(Ty::Str.is_boxed());
        assert!(Ty::List(Box::new(Ty::Int)).is_boxed());
        assert!(Ty::Aggregate(LayoutId(0)).is_boxed());
    }
}
