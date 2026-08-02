//! Diagnostic codes produced by the parser.
//!
//! The parser owns the `DEED2xxx` range. Codes are stable and never reused.

/// A token that cannot appear where it was found.
pub const UNEXPECTED_TOKEN: &str = "DEED2001";

/// A file that does not begin with a `module` declaration.
pub const MISSING_MODULE_DECLARATION: &str = "DEED2002";

/// A token at the top level that cannot begin a declaration.
pub const EXPECTED_DECLARATION: &str = "DEED2003";

/// A contract clause given twice on one function.
pub const DUPLICATE_CONTRACT_CLAUSE: &str = "DEED2004";

/// An `ensures` obligation whose outcome is neither `ok` nor `err`.
///
/// Also the clause that names no outcome at all, where the condition itself
/// stands where `ok =>` belongs. It is the same gap seen from the other side,
/// and it used to be reported twice: once for the word in the outcome's place
/// and once for the `=>` that never came. Now the parser reads the rest as the
/// condition it plainly is, and there is one sentence and one repair.
pub const INVALID_ENSURES_OUTCOME: &str = "DEED2005";

/// Contract clauses written in an order other than `where`, `uses`, `ensures`.
///
/// P4 says there is one canonical form. Clause order is part of it: a signature
/// is the review surface, and it should read the same way every time.
pub const CONTRACT_CLAUSE_ORDER: &str = "DEED2006";

/// A function parameter written without a type.
///
/// P5 says nothing implicit crosses a boundary, and a parameter is the
/// boundary. An untyped one used to become the unknown type, which agrees with
/// everything, so every mistake made with it was invisible and a closure could
/// carry any effect through it into a function that declared none.
pub const MISSING_PARAMETER_TYPE: &str = "DEED2007";

/// A choice variant given its payload by position rather than by name.
///
/// `Circle(Int)` is what anyone arriving from a language with tuple variants
/// writes first, and it is refused. Saying so as "expected `}`" reads as a
/// typo in a line that has none, so it has a code of its own.
///
/// Whether it should be refused at all is open. `ok` and `err` carry a value
/// positionally and are built in, which is the shortcut `design/02-syntax.md`
/// records under what holds `Result` in the language.
pub const POSITIONAL_VARIANT: &str = "DEED2008";

/// A word in front of a `let` name, such as `mut`, that the language has no
/// place for.
///
/// It has a code of its own because taking `mut` as the name is the reading
/// that produces the most and the least useful messages of any single word a
/// newcomer writes, and none of them names the word.
pub const NO_BINDING_MODIFIER: &str = "DEED2009";

/// A binding written without `let`, either behind another language's keyword
/// (`var n = 1`) or behind its type (`Int n = 1`).
///
/// Both used to arrive as a name nobody declared and an assignment to a name
/// nobody declared, which is two messages about the halves and none about the
/// line. The shape is safe to read because two names in a row on one line is
/// not a statement here.
pub const BINDING_WITHOUT_LET: &str = "DEED2010";

/// `0..10`. A range, which this language does not have.
///
/// The two dots used to be left where they were, and the rest of the file paid
/// for it: `for i in 0..10` produced six diagnostics and the next declaration
/// was reported as not being one. Two dots in a row are never anything else,
/// since a field access has a name between them, so the shape can be read and
/// swallowed whole.
pub const NO_RANGE: &str = "DEED2011";

/// `n as String`. A cast, which this language does not have.
///
/// `as` is an ordinary name, so the resolver used to answer it with "cannot
/// find `as` in this scope", which is true of a word nobody wrote as a name.
/// The conversions that exist are calls, and a call says in its return type
/// whether it can fail, which is the thing a cast is for hiding.
pub const NO_CAST: &str = "DEED2012";

/// An edition declaration the parser does not recognize.
///
/// Editions are accepted per module, and an unknown one should fail at the
/// declaration line with a list of the versions that exist.
pub const UNKNOWN_EDITION: &str = "DEED2013";

/// `spawn(f())`. A detached spawn, which this language does not have.
///
/// Deed uses structured concurrency: a task is tied to the block that started
/// it and cannot outlive it. A detached spawn leaves a task running after the
/// block that created it exits, with no clear owner and no scoped lifetime.
/// That shape is refused rather than allowed to leak.
///
/// The pattern is detected at `spawn(expr)` where `spawn` is an identifier at
/// statement level followed by an argument list on the same line.
pub const NO_DETACHED_SPAWN: &str = "DEED2014";

/// A comma-separated list written one to a line with no commas.
///
/// A separator nobody wrote is a whole file's worth of complaints. In a match
/// the first arm swallows the rest of it, the arms after are read as
/// statements, and what comes back is an inexhaustive match, an unread value
/// per arm, an expected `}`, an expected expression at each `=>` and a
/// declaration that is a closing brace. Nine diagnostics for one comma, and
/// none of them said comma. A `choice` written the same way says "insert `}`",
/// which is an answer to a question nobody asked.
///
/// Reported where the comma should have gone rather than where the parser
/// noticed, and it goes on reading the list afterwards, so three missing
/// commas is three of these rather than one and then rubble.
///
/// Three sentences under one code, one per list: arms, variants and fields.
/// They are the same mistake and the same repair, and the word that changes is
/// the name of the thing being separated.
pub const MISSING_COMMA: &str = "DEED2015";

/// A match arm written with `->` instead of `=>`.
///
/// Both arrows are in the language and they are a line apart: `->` is the one
/// in a signature, before the type a function hands back, and `=>` is the one
/// in an arm and in an obligation. Reaching for the wrong one is a slip of the
/// hand rather than a misunderstanding, and it used to cost four diagnostics,
/// because the arm ended at the pattern and the body after it was read as a
/// statement of the match's block.
///
/// So it is named, repaired and stepped over, and the arm goes on being read
/// as an arm.
pub const WRONG_ARROW: &str = "DEED2016";

/// A constraint written on a parameter rather than in the function's `where`.
///
/// A `type` carries its refinement inline (`type InStock = Int where value >
/// 0`), so writing the same thing on a parameter is a fair guess rather than a
/// misunderstanding. A function carries its constraints in the clause after
/// the signature instead, which is the only place every parameter is in scope
/// at once, and `restock(count: Int where count + delivered > 0, ...)` is the
/// case that makes the difference obvious.
///
/// Named, and then put where it belongs: the expression is read into the
/// function's `where` clause, so the names in it resolve and the four
/// diagnostics this used to cost come down to this one.
pub const PARAMETER_CONSTRAINT: &str = "DEED2017";

/// `xs ++ ys` or `x :: xs`: an operator borrowed from a language that has one.
///
/// Both are doubled, and nothing in this grammar puts two `+` or two `:` in a
/// row, so the shape can be read where it is written rather than left to fall
/// apart. Left alone, `++` costs an expected expression and `::` costs an
/// unread value and two more, none of which mention a list.
///
/// Two sentences under one code, because it is one mistake: this language
/// builds lists by calling something. The sentence that changes names the
/// call, `concat` for one and `prepend` for the other, and says the list goes
/// first, which is the part that is easy to get backwards.
pub const NO_LIST_OPERATOR: &str = "DEED2018";

/// `state count: Int = 0`: a handler's state given its value in the handler.
///
/// The value comes from the `with` that installs the handler, which is what
/// lets one handler be installed twice from two different starting points.
/// Written on the declaration, it used to end the handler: the state stopped
/// at the type, the `=` was not a member, and what came back was seven
/// diagnostics, one of them saying the handler implements none of its
/// operations and four about names in the operations the parser never reached.
///
/// Read and dropped, so the operations after it are still operations, and the
/// note says both halves of the shape rather than which token was wanted.
pub const STATE_INITIALISER: &str = "DEED2019";

/// `a and b`, `a or b`: the words other languages spell their operators with.
///
/// The resolver already had an answer for these, and it never got to give it.
/// `and` is an ordinary name here, so a condition or a contract clause holding
/// one stops at the word: the expression ends, and the reader is told a block
/// was expected. Nothing in that sentence is the word they wrote.
///
/// Read as the operator they mean, so the line is the program they wrote and
/// the rest of it is checked. The repair is a word for a symbol, which is the
/// whole of it, so it is applied rather than offered.
pub const WORD_OPERATOR: &str = "DEED2020";
