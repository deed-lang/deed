//! Recursive descent for declarations, Pratt for expressions.
//!
//! The parser answers one question, "is this a well formed program", and
//! refuses to answer any others. It does not know what a name means, and where
//! the grammar is genuinely ambiguous about that it produces one node and lets
//! name resolution decide, rather than guessing earlier than the information
//! arrives.
//!
//! Recovery follows the same rule as the lexer: never stop at the first
//! problem. A syntax error inside one function must not prevent the next
//! function from being parsed, because each hidden error costs a round trip.

use deed_ast::{
    Accumulator, BinaryOp, Block, ChoiceDecl, Contract, DeprecateDecl, EditionDecl, EffectDecl,
    EffectRef, Ensures, Expr, FieldDecl, FieldInit, FnDecl, FnSig, HandlerDecl, Ident, Item,
    MatchArm, Module, ModulePath, Outcome, Param, Pattern, PatternField, RecordDecl, Stmt,
    TestDecl, Type, TypeAlias, UnaryOp, Use, Variant,
};
use deed_diagnostics::{Applicability, Diagnostic, FileId, Span, SuggestedEdit};
use deed_lexer::{Keyword, Token, TokenKind};

use crate::codes;

/// The result of parsing one file.
pub struct Parsed {
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parsed {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Words the parser reads in one position but which are not keywords.
///
/// `state` opens a handler field, `at` names the index of a `for`, `while` is
/// the condition a `for` stops on, `refuses` is the marker on the `assert`
/// that expects a contract to turn something down, and `ok` and `err` are the
/// outcome an `ensures` clause is about. None of them is reserved: a variable
/// may still be called `at`, which is why they are read with `eat_named`
/// rather than lexed. An editor still has to colour them, so the set is
/// written down here rather than as string literals scattered through the
/// file.
///
/// What that costs, since it is the question this list keeps raising: the
/// editor grammar is a TextMate grammar and has no positions, so a word in
/// here is coloured everywhere it appears. `at(items, 0)` and `ok(value)` are
/// calls to prelude functions and are coloured like the markers they share a
/// spelling with. Being a prelude name is therefore not what keeps a word out
/// of this list, and it never was: `at` has been in it and coloured at every
/// call site since the grammar was written.
///
/// It is also less of a lie than it looks. `ok` is not an ordinary name in
/// any of its three positions. `ensures ok =>` produces an `Outcome` and
/// resolves to nothing, `ok(v)` is a pattern head no declared variant can
/// occupy, and `ok(value)` is the one call exempt from the rule that every
/// type parameter appears in a parameter type. All three are the language
/// talking about itself, and the word naming the same thing in each is why
/// they are spelled alike. The grammar cannot tell them apart, and it is
/// lexical on purpose, because the moment somebody is looking at a file is
/// the moment it does not parse.
///
/// What is left out, so the next person does not have to work it out again.
/// `mut`, `as`, `struct` and the rest of the words another language would
/// have used are read by name too, but every one of them is on a path that
/// emits a diagnostic, so none of them survives into a program worth
/// colouring. `Fn` is the only word read here that does appear in an accepted
/// program and is still not in this list, and the reason is that the grammar
/// already colours it: every type is written with a capital and the grammar
/// matches that shape. The test in `crates/deed-parser/tests/grammar.rs`
/// walks alternation groups, so a word that is coloured by shape rather than
/// by name is neither missing from the grammar nor invented by it.
pub const SOFT_KEYWORDS: [&str; 6] = ["state", "at", "while", "refuses", "ok", "err"];

/// Parses a token stream. Always produces a module, possibly containing error nodes.
pub fn parse(file: FileId, tokens: &[Token]) -> Parsed {
    Parser {
        file,
        tokens,
        pos: 0,
        diagnostics: Vec::new(),
        struct_lit: StructLit::Allow,
        last_error_at: None,
    }
    .parse_module()
}

/// Whether a `{` after an expression starts a struct literal or a block.
///
/// This is the one genuinely ambiguous corner of the grammar. `if x { ... }`
/// and `Point { x: 1 }` both look like an expression followed by a brace.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StructLit {
    /// Ordinary expression position. A brace after a path is a struct literal.
    Allow,
    /// Condition and scrutinee position. A brace always starts a block.
    Deny,
    /// Handler position in `with`, and the iterable and accumulator of a
    /// `for`. A brace is a struct literal when it is followed by `name:`,
    /// which tells `InMemoryLedger { a: 1 }` from the block that comes after
    /// the handler list, or when it is an empty pair with a block behind it,
    /// which is the only way a record with no fields can be written here.
    RequireColon,
}

/// Contract clauses have one canonical order, and this is it.
fn clause_rank(kw: Keyword) -> u8 {
    match kw {
        Keyword::Where => 0,
        Keyword::Uses => 1,
        Keyword::Ensures => 2,
        _ => u8::MAX,
    }
}

/// Whether a parameter list is the only place its types could be written.
///
/// A handler operation is the one place it is not, because the effect it
/// implements already declared the whole signature and making the handler
/// repeat it would be redundancy nothing checks. Everywhere else, including a
/// closure, leaving the type out means nobody knows it.
///
/// Closures were briefly the other exception, on the grounds that a closure
/// cannot leave the function that wrote it so its parameters are not a boundary
/// anyone reviews. That is true and it is a different claim from "may be
/// unchecked": the parameters were the unknown type, so the closure's body was
/// not checked at all.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypesRequired {
    Yes,
    No,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edition {
    E2024,
    E2025,
}

impl Edition {
    fn from_decl(edition: Option<&EditionDecl>) -> Self {
        match edition.map(|decl| decl.year) {
            Some(2025) => Edition::E2025,
            _ => Edition::E2024,
        }
    }

    fn allows_use_semicolon(self) -> bool {
        matches!(self, Edition::E2025)
    }
}

/// Words the language spells differently, and words it does not have.
///
/// The first thing anybody writes in a new language is the thing they wrote in
/// the last one. Answering `struct` with a list of the seven declaration forms
/// is correct and slow. Answering it with `record` is what was asked.
///
/// Only words with one honest answer are here. `trait` and `interface` are
/// near `effect` and near nothing, `impl` is near `handler` and near a record
/// literal, and guessing on those would put a word in somebody's file that
/// does not mean what they meant.
enum Elsewhere {
    /// The same idea under another name.
    SpelledHere(&'static str),
    /// A form this language decided not to have, and why.
    NotAThing(&'static str),
}

fn spelled_elsewhere(word: &str) -> Option<Elsewhere> {
    Some(match word {
        "struct" | "class" => Elsewhere::SpelledHere("record"),
        "enum" => Elsewhere::SpelledHere("choice"),
        "import" | "include" | "require" => Elsewhere::SpelledHere("use"),
        "func" | "function" | "def" | "fun" => Elsewhere::SpelledHere("fn"),
        "pub" | "public" | "export" => Elsewhere::NotAThing(
            "every declaration is exported and there is no visibility modifier, because a \
             language with no wildcard imports already shows the reader every name a file \
             pulled in",
        ),
        "const" | "var" | "val" => Elsewhere::NotAThing(
            "a file holds declarations rather than statements, so a named value at the top \
             level is a `fn` that returns it",
        ),
        _ => return None,
    })
}

/// A word in front of a `let` name that another language would have accepted.
///
/// Only the words that really follow a binding keyword elsewhere are here.
/// `var n = 1` and `const n = 1` are written without a `let` in front, so they
/// arrive somewhere else entirely, which is `declared_elsewhere` below.
fn binding_modifier(word: &str) -> bool {
    matches!(word, "mut" | "mutable")
}

/// How `word name = value` was meant to be read.
///
/// The statement is not one this language has either way. What this decides is
/// which sentence to say about it, and both sentences end in `let`.
enum Declared {
    /// `var n = 1`. Another language's binding keyword, and whether it asked
    /// for something that can be assigned to again.
    Keyword { rebindable: bool },
    /// `Int n = 1`. The type in front of the name rather than after it.
    TypeFirst,
}

fn declared_elsewhere(word: &str) -> Option<Declared> {
    match word {
        "var" | "local" => Some(Declared::Keyword { rebindable: true }),
        "const" | "val" => Some(Declared::Keyword { rebindable: false }),
        // Every type in the language is written with a capital and every value
        // without one, so an initial capital is what tells `Int n = 1` from
        // two names that have nothing to do with each other. The parser cannot
        // ask what `Int` resolves to and does not need to: the shape is
        // already wrong, and a word that is neither of these keeps the answer
        // it gets today rather than being guessed at.
        _ if word.starts_with(char::is_uppercase) => Some(Declared::TypeFirst),
        _ => None,
    }
}

struct Parser<'a> {
    file: FileId,
    tokens: &'a [Token],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    struct_lit: StructLit,
    /// Position of the last reported error, used to suppress the cascade that
    /// otherwise follows a single mistake.
    last_error_at: Option<usize>,
}

impl<'a> Parser<'a> {
    // -- token helpers -----------------------------------------------------

    fn peek(&self) -> &Token {
        // The stream always ends with `Eof`, so this never goes out of bounds.
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn nth_kind(&self, n: usize) -> &TokenKind {
        let index = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[index].kind
    }

    fn nth(&self, n: usize) -> &Token {
        let index = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[index]
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.kind() == kind
    }

    fn at_kw(&self, kw: Keyword) -> bool {
        matches!(self.kind(), TokenKind::Keyword(k) if *k == kw)
    }

    fn at_eof(&self) -> bool {
        matches!(self.kind(), TokenKind::Eof)
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !self.at_eof() {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Keyword) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consumes an ordinary name when it is the one expected.
    ///
    /// For a word that means something in one position and is a name
    /// everywhere else. Reserving such a word for the whole language costs a
    /// name people want, and the position it matters in has nothing else it
    /// could be.
    fn eat_named(&mut self, name: &str) -> bool {
        if matches!(self.kind(), TokenKind::Ident(found) if found == name) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn emit(&mut self, diagnostic: Diagnostic) {
        // One mistake should produce one diagnostic. Everything derived from it
        // is noise that buries the line actually needing an edit.
        if self.last_error_at == Some(self.pos) {
            return;
        }
        self.last_error_at = Some(self.pos);
        self.diagnostics.push(diagnostic);
    }

    /// Consumes `kind`, or reports what was expected and leaves the stream alone.
    fn expect(&mut self, kind: TokenKind, context: &str) -> Option<Token> {
        if self.at(&kind) {
            return Some(self.bump());
        }

        let span = self.span();
        let expected = kind.describe();
        let found = self.kind().describe();
        let mut diagnostic = Diagnostic::error(
            codes::UNEXPECTED_TOKEN,
            self.file,
            span,
            format!("expected {expected} while parsing {context}, found {found}"),
        )
        .with_primary_label(format!("expected {expected}"));

        // Missing closing delimiters are the common case and the fix is never
        // in doubt about what to insert, only about where.
        if matches!(
            kind,
            TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket
        ) {
            diagnostic = diagnostic.with_fix(
                format!("insert {expected}"),
                Span::at(span.start),
                kind.describe().trim_matches('`').to_string(),
                Applicability::MaybeIncorrect,
            );
        }

        self.emit(diagnostic);
        None
    }

    fn expect_ident(&mut self, context: &str) -> Option<Ident> {
        if let TokenKind::Ident(name) = self.kind() {
            let name = name.clone();
            let token = self.bump();
            return Some(Ident::new(name, token.span));
        }

        let span = self.span();
        let found = self.kind().describe();
        self.emit(
            Diagnostic::error(
                codes::UNEXPECTED_TOKEN,
                self.file,
                span,
                format!("expected a name while parsing {context}, found {found}"),
            )
            .with_primary_label("expected a name"),
        );
        None
    }

    // -- recovery ----------------------------------------------------------

    fn at_item_start(&self) -> bool {
        matches!(
            self.kind(),
            TokenKind::Keyword(
                Keyword::Type
                    | Keyword::Record
                    | Keyword::Choice
                    | Keyword::Effect
                    | Keyword::Handler
                    | Keyword::Fn
                    | Keyword::Test
                    | Keyword::Module
                    | Keyword::Use
            )
        )
    }

    /// Skips forward to something that can start a declaration.
    ///
    /// Braces are tracked so that a `fn` nested inside a handler body does not
    /// look like the start of a new top level item.
    fn synchronize_item(&mut self) {
        let mut depth = 0i32;
        while !self.at_eof() {
            match self.kind() {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => depth -= 1,
                _ if depth <= 0 && self.at_item_start() => return,
                _ => {}
            }
            self.bump();
        }
    }

    // -- module ------------------------------------------------------------

    fn parse_module(mut self) -> Parsed {
        let start = self.span();

        let name = if self.at_kw(Keyword::Module) {
            self.bump();
            self.parse_module_path("a module declaration")
        } else {
            self.emit(
                Diagnostic::error(
                    codes::MISSING_MODULE_DECLARATION,
                    self.file,
                    Span::at(start.start),
                    "every file must begin with a `module` declaration",
                )
                .with_primary_label("expected `module` here")
                .with_note("the module path is the file's identity, so nothing has to keep a name and a path in sync"),
            );
            None
        };
        let edition = self.parse_edition_decl();
        let language = Edition::from_decl(edition.as_ref());

        let mut uses = Vec::new();
        while self.at_kw(Keyword::Use) {
            let before = self.pos;
            self.bump();
            match self.parse_use() {
                Some(item) => uses.push(item),
                None => self.synchronize_item(),
            }
            if self.at(&TokenKind::Semi) {
                let semi = self.bump();
                if !language.allows_use_semicolon() {
                    self.emit(
                        Diagnostic::error(
                            codes::UNEXPECTED_TOKEN,
                            self.file,
                            semi.span,
                            "a `use` declaration cannot end with `;` in this edition",
                        )
                        .with_primary_label("remove `;`")
                        .with_note(
                            "`edition 2025` accepts `;` after a `use`; earlier editions do not",
                        ),
                    );
                }
            }
            if self.pos == before {
                self.bump();
            }
        }

        let mut items = Vec::new();
        while !self.at_eof() {
            let before = self.pos;
            match self.parse_item() {
                Some(item) => items.push(item),
                None => self.synchronize_item(),
            }
            if self.pos == before {
                self.bump();
            }
        }

        let end = self.span();
        Parsed {
            module: Module {
                name,
                edition,
                uses,
                items,
                span: start.to(end),
            },
            diagnostics: self.diagnostics,
        }
    }

    fn parse_edition_decl(&mut self) -> Option<EditionDecl> {
        if !self.eat_named("edition") {
            return None;
        }

        let span = self.span();
        let TokenKind::Int(year) = self.kind() else {
            self.emit(
                Diagnostic::error(
                    codes::UNEXPECTED_TOKEN,
                    self.file,
                    span,
                    format!(
                        "expected an edition year after `edition`, found {}",
                        self.kind().describe()
                    ),
                )
                .with_primary_label("expected an edition year")
                .with_note("supported editions are `2024` and `2025`"),
            );
            return None;
        };

        let year = *year;
        let token = self.bump();
        if matches!(year, 2024 | 2025) {
            return Some(EditionDecl {
                year: year as u32,
                span: token.span,
            });
        }

        self.emit(
            Diagnostic::error(
                codes::UNKNOWN_EDITION,
                self.file,
                token.span,
                format!("unknown edition `{year}`"),
            )
            .with_primary_label("unknown edition")
            .with_note("supported editions are `2024` and `2025`"),
        );
        None
    }

    fn parse_module_path(&mut self, context: &str) -> Option<ModulePath> {
        let first = self.expect_ident(context)?;
        let start = first.span;
        let mut segments = vec![first];
        while self.eat(&TokenKind::Slash) {
            match self.expect_ident(context) {
                Some(segment) => segments.push(segment),
                None => break,
            }
        }
        let span = start.to(segments.last().map(|s| s.span).unwrap_or(start));
        Some(ModulePath { segments, span })
    }

    /// `use std/result.{Result, ok, err}`
    fn parse_use(&mut self) -> Option<Use> {
        let path = self.parse_module_path("a `use` declaration")?;
        self.expect(TokenKind::Dot, "a `use` declaration")?;
        self.expect(TokenKind::LBrace, "an import list")?;

        let mut names = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            match self.expect_ident("an import list") {
                Some(name) => names.push(name),
                None => break,
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "an import list");
        Some(Use {
            span: path.span.to(end),
            path,
            names,
        })
    }

    // -- items -------------------------------------------------------------

    fn parse_item(&mut self) -> Option<Item> {
        let TokenKind::Keyword(kw) = self.kind() else {
            let span = self.span();
            let found = self.kind().describe();
            let elsewhere = match self.kind() {
                TokenKind::Ident(name) => spelled_elsewhere(name),
                _ => None,
            };

            let mut diagnostic = Diagnostic::error(
                codes::EXPECTED_DECLARATION,
                self.file,
                span,
                format!("expected a declaration, found {found}"),
            )
            .with_primary_label("not the start of a declaration");

            // Somebody who wrote `struct` did not make a mistake about this
            // language, they made an assumption from another one, and the
            // list of every declaration form is a slower way to answer them
            // than the one word they were reaching for.
            match elsewhere {
                Some(Elsewhere::SpelledHere(word)) => {
                    diagnostic = diagnostic
                        .with_note(format!("this language spells it `{word}`"))
                        .with_fix(
                            format!("write `{word}`"),
                            span,
                            word,
                            Applicability::MachineApplicable,
                        );
                }
                Some(Elsewhere::NotAThing(note)) => diagnostic = diagnostic.with_note(note),
                None => {
                    diagnostic = diagnostic.with_note(
                        "a file contains `deprecated`, `type`, `record`, `choice`, `effect`, `handler`, `fn` and `test` declarations",
                    );
                }
            }

            self.emit(diagnostic);
            return None;
        };

        match kw {
            Keyword::Deprecated => self.parse_deprecate().map(Item::Deprecate),
            Keyword::Type => self.parse_type_alias().map(Item::TypeAlias),
            Keyword::Record => self.parse_record().map(Item::Record),
            Keyword::Choice => self.parse_choice().map(Item::Choice),
            Keyword::Effect => self.parse_effect().map(Item::Effect),
            Keyword::Handler => self.parse_handler().map(Item::Handler),
            Keyword::Fn => self.parse_fn(TypesRequired::Yes).map(Item::Function),
            Keyword::Test => self.parse_test().map(Item::Test),
            _ => {
                let span = self.span();
                let found = self.kind().describe();
                self.emit(
                    Diagnostic::error(
                        codes::EXPECTED_DECLARATION,
                        self.file,
                        span,
                        format!("expected a declaration, found {found}"),
                    )
                    .with_primary_label("not the start of a declaration"),
                );
                None
            }
        }
    }

    /// `deprecated old_name -> new_name`
    fn parse_deprecate(&mut self) -> Option<DeprecateDecl> {
        let start = self.bump().span;
        let old = self.expect_ident("a deprecation declaration")?;
        self.expect(TokenKind::Arrow, "a deprecation declaration")?;
        let new = self.expect_ident("a deprecation declaration")?;
        Some(DeprecateDecl {
            span: start.to(new.span),
            old,
            new,
        })
    }

    /// `type Positive = Int where value > 0`
    fn parse_type_alias(&mut self) -> Option<TypeAlias> {
        let start = self.bump().span;
        let name = self.expect_ident("a type alias")?;
        let generics = self.parse_type_params();
        self.expect(TokenKind::Eq, "a type alias")?;
        let ty = self.parse_type();

        let refinement = if self.eat_kw(Keyword::Where) {
            Some(self.parse_expr_no_struct())
        } else {
            None
        };

        let end = refinement
            .as_ref()
            .map(Expr::span)
            .unwrap_or_else(|| ty.span());
        Some(TypeAlias {
            name,
            generics,
            ty,
            refinement,
            span: start.to(end),
        })
    }

    fn parse_record(&mut self) -> Option<RecordDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("a record declaration")?;
        let generics = self.parse_type_params();
        let (fields, end) = self.parse_field_block("a record declaration")?;
        Some(RecordDecl {
            name,
            generics,
            fields,
            span: start.to(end),
        })
    }

    /// `<T, U>`, or nothing at all.
    ///
    /// Only ever read in a declaration, right after the name, where the `<`
    /// cannot be a comparison. That is what keeps type arguments out of
    /// expression position and with them the `f<a>(b)` versus `f < a > (b)`
    /// ambiguity that costs other parsers a real amount of lookahead.
    fn parse_type_params(&mut self) -> Vec<Ident> {
        self.parse_declaration_params().0
    }

    /// The same list, which may also hold row variables written `uses r`.
    ///
    /// One list rather than two, because a reader wants to see everything a
    /// call has to work out in one place, and `uses` marks which kind each one
    /// is rather than leaving it to be inferred from where it turns up.
    fn parse_declaration_params(&mut self) -> (Vec<Ident>, Vec<Ident>) {
        let mut generics = Vec::new();
        let mut rows = Vec::new();
        if !self.eat(&TokenKind::Lt) {
            return (generics, rows);
        }
        while !self.at(&TokenKind::Gt) && !self.at_eof() {
            let before = self.pos;
            let is_row = self.at_kw(Keyword::Uses);
            if is_row {
                self.bump();
            }
            let Some(parameter) = self.expect_ident(if is_row {
                "a row variable"
            } else {
                "a type parameter"
            }) else {
                break;
            };
            if is_row {
                rows.push(parameter);
            } else {
                generics.push(parameter);
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }
        self.expect(TokenKind::Gt, "a type parameter list");
        (generics, rows)
    }

    /// `{ name: Type, other: Type }`, trailing comma allowed.
    fn parse_field_block(&mut self, context: &str) -> Option<(Vec<FieldDecl>, Span)> {
        self.expect(TokenKind::LBrace, context)?;
        let mut fields = Vec::new();

        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            let Some(name) = self.expect_ident(context) else {
                break;
            };
            if self.expect(TokenKind::Colon, context).is_none() {
                break;
            }
            let ty = self.parse_type();
            fields.push(FieldDecl {
                span: name.span.to(ty.span()),
                name,
                ty,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, context);
        Some((fields, end))
    }

    /// A variant written `Circle(Int)`, which this language does not have.
    ///
    /// Reported here rather than left to `expect`, which would say "expected
    /// `}`" and send somebody looking for a missing brace in a line that has
    /// none. Anyone arriving from a language with tuple variants writes this
    /// first, so it is worth a sentence rather than a token name.
    ///
    /// The payload is skipped so that the rest of the declaration still parses
    /// and the reader gets the whole file's worth of errors rather than this
    /// one and a cascade behind it.
    fn positional_variant(&mut self, name: &Ident) -> Span {
        let open = self.bump().span;

        let mut depth = 1usize;
        let mut end = open;
        while depth > 0 && !self.at_eof() {
            match self.kind() {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth -= 1,
                // An unclosed `(` is a different mistake, and running to the
                // end of the file looking for its partner would bury it.
                TokenKind::RBrace => break,
                _ => {}
            }
            end = self.bump().span;
        }

        self.diagnostics.push(
            Diagnostic::error(
                codes::POSITIONAL_VARIANT,
                self.file,
                open.to(end),
                format!("`{}` carries its payload by position", name.name),
            )
            .with_primary_label("a variant's fields are named")
            .with_note(
                "a variant is written `Variant { field: Type }`, with a name chosen for every \
                 field, and it is matched the same way",
            )
            .with_note(
                "`ok` and `err` are the exception and they are built into the language rather than declared, \
                 which is what keeps `Result` in it",
            ),
        );

        name.span.to(end)
    }

    fn parse_choice(&mut self) -> Option<ChoiceDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("a choice declaration")?;
        let generics = self.parse_type_params();
        self.expect(TokenKind::LBrace, "a choice declaration")?;

        let mut variants = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            let Some(variant_name) = self.expect_ident("a choice variant") else {
                break;
            };

            let (fields, end) = if self.at(&TokenKind::LBrace) {
                match self.parse_field_block("a choice variant") {
                    Some((fields, end)) => (Some(fields), end),
                    None => (None, variant_name.span),
                }
            } else if self.at(&TokenKind::LParen) {
                (None, self.positional_variant(&variant_name))
            } else {
                (None, variant_name.span)
            };

            variants.push(Variant {
                span: variant_name.span.to(end),
                name: variant_name,
                fields,
            });

            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "a choice declaration");
        Some(ChoiceDecl {
            name,
            generics,
            variants,
            span: start.to(end),
        })
    }

    fn parse_effect(&mut self) -> Option<EffectDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("an effect declaration")?;
        self.expect(TokenKind::LBrace, "an effect declaration")?;

        let mut operations = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            if self.at_kw(Keyword::Fn) {
                if let Some(sig) = self.parse_fn_sig(TypesRequired::Yes) {
                    operations.push(sig);
                }
            } else {
                let span = self.span();
                let found = self.kind().describe();
                self.emit(
                    Diagnostic::error(
                        codes::UNEXPECTED_TOKEN,
                        self.file,
                        span,
                        format!("expected an operation signature, found {found}"),
                    )
                    .with_primary_label("expected `fn`")
                    .with_note(
                        "an effect declares operations and nothing else, so it has no bodies",
                    ),
                );
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "an effect declaration");
        Some(EffectDecl {
            name,
            operations,
            span: start.to(end),
        })
    }

    fn parse_handler(&mut self) -> Option<HandlerDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("a handler declaration")?;
        self.expect(
            TokenKind::Keyword(Keyword::Implements),
            "a handler declaration",
        )?;
        let effect = self.expect_ident("a handler declaration")?;
        self.expect(TokenKind::LBrace, "a handler declaration")?;

        let mut state = Vec::new();
        let mut operations = Vec::new();

        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            // A name rather than a keyword. `state` means something here and
            // nowhere else, and the only other thing a member can start with
            // is `fn`, so there is nothing to disambiguate and no reason to
            // reserve the word for the rest of the language.
            if self.eat_named("state") {
                if let Some(field_name) = self.expect_ident("handler state") {
                    if self.expect(TokenKind::Colon, "handler state").is_some() {
                        let ty = self.parse_type();
                        state.push(FieldDecl {
                            span: field_name.span.to(ty.span()),
                            name: field_name,
                            ty,
                        });
                    }
                }
            } else if self.at_kw(Keyword::Fn) {
                if let Some(function) = self.parse_fn(TypesRequired::No) {
                    operations.push(function);
                }
            } else {
                let span = self.span();
                let found = self.kind().describe();
                self.emit(
                    Diagnostic::error(
                        codes::UNEXPECTED_TOKEN,
                        self.file,
                        span,
                        format!("expected `state` or `fn` in a handler, found {found}"),
                    )
                    .with_primary_label("expected `state` or `fn`"),
                );
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "a handler declaration");
        Some(HandlerDecl {
            name,
            effect,
            state,
            operations,
            span: start.to(end),
        })
    }

    fn parse_test(&mut self) -> Option<TestDecl> {
        let start = self.bump().span;
        let (name, name_span) = match self.kind() {
            TokenKind::Str(value) => {
                let value = value.clone();
                let token = self.bump();
                (value, token.span)
            }
            _ => {
                let span = self.span();
                let found = self.kind().describe();
                self.emit(
                    Diagnostic::error(
                        codes::UNEXPECTED_TOKEN,
                        self.file,
                        span,
                        format!("expected a test name, found {found}"),
                    )
                    .with_primary_label("expected a string")
                    .with_note(
                        "tests are named with a sentence, as in `test \"refuses to overdraw\"`",
                    ),
                );
                return None;
            }
        };

        let body = self.parse_block();
        Some(TestDecl {
            name,
            name_span,
            span: start.to(body.span),
            body,
        })
    }

    // -- functions ---------------------------------------------------------

    fn parse_fn_sig(&mut self, types_required: TypesRequired) -> Option<FnSig> {
        let start = self.bump().span;
        let name = self.expect_ident("a function signature")?;

        // `<T, U, uses r>`, and only here. In a declaration the `<` cannot be
        // a comparison, so this needs no lookahead and none of the machinery
        // that `f<a>(b)` in expression position would.
        let (generics, rows) = self.parse_declaration_params();

        self.expect(TokenKind::LParen, "a parameter list")?;

        let mut params = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.at_eof() {
            let before = self.pos;
            let Some(param_name) = self.expect_ident("a parameter") else {
                break;
            };
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type())
            } else {
                // P5: nothing implicit crosses a boundary, and a parameter is
                // the boundary. Reported rather than refused, so the rest of
                // the file still parses and the author sees every mistake in
                // one pass instead of one per run.
                if types_required == TypesRequired::Yes {
                    self.emit(
                        Diagnostic::error(
                            codes::MISSING_PARAMETER_TYPE,
                            self.file,
                            param_name.span,
                            format!("`{}` has no type", param_name.name),
                        )
                        .with_primary_label("a parameter needs a type")
                        .with_note(
                            "a signature is what a reviewer is entitled to stop at, so a parameter with no type is a hole in it",
                        ),
                    );
                }
                None
            };
            params.push(Param {
                span: param_name
                    .span
                    .to(ty.as_ref().map(Type::span).unwrap_or(param_name.span)),
                name: param_name,
                ty,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let mut end = self.span();
        self.expect(TokenKind::RParen, "a parameter list");

        let ret = if self.eat(&TokenKind::Arrow) {
            let ty = self.parse_type();
            end = ty.span();
            Some(ty)
        } else {
            None
        };

        Some(FnSig {
            name,
            generics,
            rows,
            params,
            ret,
            span: start.to(end),
        })
    }

    fn parse_fn(&mut self, types_required: TypesRequired) -> Option<FnDecl> {
        let sig = self.parse_fn_sig(types_required)?;
        let contract = self.parse_contract();
        let body = self.parse_block();
        Some(FnDecl {
            span: sig.span.to(body.span),
            sig,
            contract,
            body,
        })
    }

    /// The contract block: `where`, then `uses`, then `ensures`, in that order.
    fn parse_contract(&mut self) -> Contract {
        let mut contract = Contract::default();
        let start = self.span();
        let mut end = start;
        let mut seen: Option<(Keyword, Span)> = None;
        let mut any = false;

        while let TokenKind::Keyword(kw) = *self.kind() {
            if !matches!(kw, Keyword::Where | Keyword::Uses | Keyword::Ensures) {
                break;
            }

            let kw_span = self.span();
            if let Some((previous, previous_span)) = seen {
                if previous == kw {
                    self.emit(
                        Diagnostic::error(
                            codes::DUPLICATE_CONTRACT_CLAUSE,
                            self.file,
                            kw_span,
                            format!("`{}` appears twice in one contract", kw.as_str()),
                        )
                        .with_primary_label("second occurrence")
                        .with_secondary(previous_span, "first one here")
                        .with_note(
                            "write all of the obligations in a single clause, separated by commas",
                        ),
                    );
                } else if clause_rank(kw) < clause_rank(previous) {
                    self.emit(
                        Diagnostic::error(
                            codes::CONTRACT_CLAUSE_ORDER,
                            self.file,
                            kw_span,
                            format!(
                                "`{}` must come before `{}`",
                                kw.as_str(),
                                previous.as_str()
                            ),
                        )
                        .with_primary_label("out of order")
                        .with_secondary(previous_span, "written after this")
                        .with_note(
                            "contract clauses are always `where`, then `uses`, then `ensures`, so that every signature reads the same way",
                        ),
                    );
                }
            }
            seen = Some((kw, kw_span));
            any = true;
            self.bump();

            match kw {
                Keyword::Where => {
                    let mut items = self.parse_contract_list(|p| p.parse_expr_no_struct());
                    contract.requires.append(&mut items);
                }
                Keyword::Uses => {
                    let mut items = self.parse_contract_list(|p| p.parse_effect_ref());
                    contract.uses.append(&mut items);
                }
                Keyword::Ensures => {
                    let mut items = self.parse_contract_list(|p| p.parse_ensures());
                    contract.ensures.append(&mut items);
                }
                _ => unreachable!("guarded above"),
            }
            end = self.span();
        }

        contract.span = any.then(|| start.to(end));
        contract
    }

    /// A contract clause ends at the body brace or at the next clause keyword.
    fn at_contract_end(&self) -> bool {
        self.at(&TokenKind::LBrace)
            || self.at_kw(Keyword::Where)
            || self.at_kw(Keyword::Uses)
            || self.at_kw(Keyword::Ensures)
            || self.at_eof()
    }

    fn parse_contract_list<T>(&mut self, mut item: impl FnMut(&mut Self) -> T) -> Vec<T> {
        let mut out = Vec::new();
        loop {
            if self.at_contract_end() {
                break;
            }
            let before = self.pos;
            out.push(item(self));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }
        out
    }

    /// `Ledger`, `Ledger.read` or `sys.*`.
    fn parse_effect_ref(&mut self) -> EffectRef {
        let Some(effect) = self.expect_ident("an effect") else {
            let span = self.span();
            return EffectRef {
                effect: Ident::new("", span),
                operation: None,
                all: false,
                span,
            };
        };

        let mut operation = None;
        let mut all = false;
        let mut end = effect.span;

        if self.eat(&TokenKind::Dot) {
            if self.at(&TokenKind::Star) {
                end = self.bump().span;
                all = true;
            } else if let Some(op) = self.expect_ident("an effect operation") {
                end = op.span;
                operation = Some(op);
            }
        }

        EffectRef {
            span: effect.span.to(end),
            effect,
            operation,
            all,
        }
    }

    /// `ok => balance(from) == old(balance(from)) - amount`
    fn parse_ensures(&mut self) -> Ensures {
        let outcome_span = self.span();

        // Nothing but these two words can stand here, and what comes out is
        // an `Outcome` rather than a name, but they are still the prelude's
        // two constructors everywhere else. So they are read by name, and
        // they are in `SOFT_KEYWORDS` with the rest of the words that are
        // syntax in one place and nothing in particular in the others.
        let outcome = if self.eat_named("ok") {
            Outcome::Ok
        } else if self.eat_named("err") {
            Outcome::Err
        } else {
            let found = self.kind().describe();
            self.emit(
                Diagnostic::error(
                    codes::INVALID_ENSURES_OUTCOME,
                    self.file,
                    outcome_span,
                    format!("expected `ok` or `err`, found {found}"),
                )
                .with_primary_label("not an outcome")
                .with_note(
                    "obligations are stated per outcome so that neither the success case nor the failure case can be left unsaid",
                ),
            );
            // Consume it anyway. Whatever it was, it stood where the outcome
            // belongs, and leaving it would derail the `=>` that follows into
            // a second diagnostic about the same mistake.
            if !self.at_eof() && !self.at(&TokenKind::FatArrow) {
                self.bump();
            }
            Outcome::Ok
        };

        self.expect(TokenKind::FatArrow, "an `ensures` obligation");
        let condition = self.parse_expr_no_struct();
        Ensures {
            outcome,
            outcome_span,
            span: outcome_span.to(condition.span()),
            condition,
        }
    }

    // -- types -------------------------------------------------------------

    fn parse_type(&mut self) -> Type {
        if self.at(&TokenKind::LParen) {
            let start = self.bump().span;
            let end = self.span();
            self.expect(TokenKind::RParen, "the unit type");
            return Type::Unit(start.to(end));
        }

        let Some(name) = self.expect_ident("a type") else {
            return Type::Error(self.span());
        };

        // `Fn(Int) -> Int`. Spelled like a type because it is one, and the
        // name is reserved for it: there is one function type in the language
        // and a second thing called `Fn` would be read as this one.
        if name.name == "Fn" && self.at(&TokenKind::LParen) {
            return self.parse_fn_type(name);
        }

        let mut end = name.span;
        let mut args = Vec::new();
        if self.eat(&TokenKind::Lt) {
            // There is no shift operator in Deed, so `Map<K, Vec<V>>` closes with
            // two separate `>` tokens and needs no special handling.
            while !self.at(&TokenKind::Gt) && !self.at_eof() {
                let before = self.pos;
                args.push(self.parse_type());
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.pos == before {
                    break;
                }
            }
            end = self.span();
            self.expect(TokenKind::Gt, "a type argument list");
        }

        Type::Named {
            span: name.span.to(end),
            name,
            args,
        }
    }

    /// The rest of `Fn(Int, Int) uses Log.note -> Int`, with the name read.
    fn parse_fn_type(&mut self, name: Ident) -> Type {
        self.expect(TokenKind::LParen, "a function type");

        let mut params = Vec::new();
        while !self.at(&TokenKind::RParen) && !self.at_eof() {
            let before = self.pos;
            params.push(self.parse_type());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }
        self.expect(TokenKind::RParen, "a function type");

        // Before the arrow, not after the return type. A declaration's own
        // contract also begins with `uses` and also follows a return type, so
        // `fn f() -> Fn(Int) -> Int uses Log.note` would be two readings of
        // the same text. Here the `->` closes the list and nothing is in doubt.
        let mut row = Vec::new();
        if self.at_kw(Keyword::Uses) {
            self.bump();
            loop {
                if self.at(&TokenKind::Arrow) || self.at_eof() {
                    break;
                }
                let before = self.pos;
                row.push(self.parse_effect_ref());
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.pos == before {
                    break;
                }
            }
        }

        // The return type is written out even when it is `()`. One way to
        // write a thing, and a function type with no arrow reads like an
        // unfinished one.
        self.expect(TokenKind::Arrow, "a function type");
        let ret = self.parse_type();

        Type::Fn {
            span: name.span.to(ret.span()),
            params,
            row,
            ret: Box::new(ret),
        }
    }

    // -- blocks and statements ---------------------------------------------

    fn parse_block(&mut self) -> Block {
        let start = self.span();
        if self.expect(TokenKind::LBrace, "a block").is_none() {
            return Block {
                stmts: Vec::new(),
                tail: None,
                span: start,
            };
        }

        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
        let mut stmts = Vec::new();
        let mut tail = None;

        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            let stmt = self.parse_stmt();
            self.eat(&TokenKind::Semi);

            match stmt {
                // A trailing expression with nothing after it is the block's value.
                Stmt::Expr(expr) if self.at(&TokenKind::RBrace) => tail = Some(Box::new(expr)),
                other => stmts.push(other),
            }

            if self.pos == before {
                self.bump();
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "a block");
        self.struct_lit = saved;

        Block {
            stmts,
            tail,
            span: start.to(end),
        }
    }

    fn parse_stmt(&mut self) -> Stmt {
        if self.at_kw(Keyword::Let) {
            let start = self.bump().span;

            // `let mut n = 1`. Nothing here is a keyword, so `mut` used to be
            // taken as the name, and the reader got six messages: an unused
            // binding called `mut` offering to rename it `_mut`, a missing
            // `=`, `n` not found twice, a stray `=`, and a `1` going nowhere.
            // None of them mentioned the word they had actually written.
            //
            // A pattern could never have been two names in a row, which is
            // what makes this safe to read, and it is the same fact that
            // `assert refuses f(x)` relies on below.
            if let TokenKind::Ident(word) = self.kind()
                && binding_modifier(word)
                && matches!(self.nth_kind(1), TokenKind::Ident(_))
            {
                let word = word.clone();
                let word_span = self.span();
                self.bump();
                let name_span = self.span();
                self.emit(
                    Diagnostic::error(
                        codes::NO_BINDING_MODIFIER,
                        self.file,
                        word_span,
                        format!("there is no `{word}`, and a `let` binds a name once"),
                    )
                    .with_primary_label("no such word")
                    .with_note(
                        "exactly one thing is mutable, a handler's `state` field, which is \
                         what lets an empty effect row mean a function cannot cause a change \
                         to anything",
                    )
                    .with_note(
                        "an accumulator is written `for n in numbers with sum = 0 { ... }`, \
                         which binds `sum` again on every turn rather than assigning to it",
                    )
                    .with_fix(
                        format!("drop `{word}`"),
                        Span::new(word_span.start, name_span.start),
                        String::new(),
                        Applicability::MachineApplicable,
                    ),
                );
            }

            let pattern = self.parse_pattern();
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type())
            } else {
                None
            };
            self.expect(TokenKind::Eq, "a `let` statement");
            let init = self.parse_expr();
            return Stmt::Let {
                span: start.to(init.span()),
                pattern,
                ty,
                init,
            };
        }

        if self.at_kw(Keyword::Return) {
            let start = self.bump().span;
            let value = if self.at(&TokenKind::RBrace) || self.at(&TokenKind::Semi) {
                None
            } else {
                Some(self.parse_expr())
            };
            let end = value.as_ref().map(Expr::span).unwrap_or(start);
            return Stmt::Return {
                value,
                span: start.to(end),
            };
        }

        if self.at_kw(Keyword::Assert) {
            let start = self.bump().span;

            // `assert refuses f(x)`. `refuses` stays an ordinary name: it is
            // the marker only when an identifier follows it, and no statement
            // could ever have been two names in a row. So `assert refuses(x)`
            // is still a call to a function somebody called `refuses`, which
            // is the direction this has to fail in. The lookahead is asked
            // before the word because `eat_named` consumes what it matches.
            if matches!(self.nth_kind(1), TokenKind::Ident(_)) && self.eat_named("refuses") {
                let subject = self.parse_expr();
                return Stmt::Refuses {
                    span: start.to(subject.span()),
                    subject,
                };
            }

            let condition = self.parse_expr();
            return Stmt::Assert {
                span: start.to(condition.span()),
                condition,
            };
        }

        // `var n = 1`, `const n = 1`, `Int n = 1`. A binding written the way
        // the last language wrote it. All three used to be read as the two
        // halves they look like, an expression statement holding one name and
        // an assignment to another, so the reader was told twice that a name
        // could not be found and never that the line wanted a `let`.
        //
        // Two names in a row is not a statement, which is what makes this safe
        // to read, and it is the same fact `let mut n = 1` and
        // `assert refuses f(x)` rest on. The line break is the other half of
        // it: `foo` on one line and `n = 1` on the next really are an
        // expression and an assignment, and that is a program.
        if let TokenKind::Ident(word) = self.kind()
            && matches!(self.nth_kind(1), TokenKind::Ident(_))
            && self.nth_kind(2) == &TokenKind::Eq
            && !self.nth(1).starts_line
            && !self.nth(2).starts_line
            && let Some(reading) = declared_elsewhere(word)
        {
            let word = word.clone();
            let word_span = self.span();
            let name_span = self.nth(1).span;
            let name = match self.nth_kind(1) {
                TokenKind::Ident(name) => name.clone(),
                _ => unreachable!("guarded by the lookahead above"),
            };

            let diagnostic = match reading {
                Declared::Keyword { rebindable } => {
                    let mut diagnostic = Diagnostic::error(
                        codes::BINDING_WITHOUT_LET,
                        self.file,
                        word_span,
                        format!("there is no `{word}`, and a binding is written `let`"),
                    )
                    .with_primary_label("no such word")
                    .with_note(
                        "a `let` binds its name once and nothing assigns to it again, so the \
                         language has no second word for the bindings that were never going \
                         to change",
                    );

                    // Only the words that asked for something the language
                    // refuses need to hear why. `const` and `val` are asking
                    // for exactly what a `let` already is.
                    if rebindable {
                        diagnostic = diagnostic
                            .with_note(
                                "exactly one thing is mutable, a handler's `state` field, \
                                 which is what lets an empty effect row mean a function \
                                 cannot cause a change to anything",
                            )
                            .with_note(
                                "an accumulator is written `for n in numbers with sum = 0 \
                                 { ... }`, which binds `sum` again on every turn rather than \
                                 assigning to it",
                            );
                    }

                    diagnostic.with_fix(
                        "write `let`",
                        word_span,
                        "let".to_string(),
                        Applicability::MachineApplicable,
                    )
                }
                Declared::TypeFirst => Diagnostic::error(
                    codes::BINDING_WITHOUT_LET,
                    self.file,
                    word_span,
                    format!(
                        "`{word}` is the type of `{name}`, and a type is written after the name"
                    ),
                )
                .with_primary_label("the type comes second")
                .with_note(
                    "a binding is written `let name: Type = value`, and the type can be left \
                     off when the value already says it",
                )
                .with_note(
                    "a signature is the one place a type has to be written, because it is \
                     the boundary somebody else reads",
                )
                .with_fix(
                    format!("write `let {name}: {word}`"),
                    word_span.to(name_span),
                    format!("let {name}: {word}"),
                    Applicability::MachineApplicable,
                ),
            };
            self.emit(diagnostic);

            // Read the rest as the `let` it was meant to be. The name is bound
            // and the initialiser is checked, so one mistake stays one
            // message instead of taking the lines below it down as well.
            self.bump();
            let pattern = self.parse_pattern();
            let ty = match reading {
                Declared::TypeFirst => Some(Type::Named {
                    name: Ident::new(word, word_span),
                    args: Vec::new(),
                    span: word_span,
                }),
                Declared::Keyword { .. } => None,
            };
            self.expect(TokenKind::Eq, "a `let` statement");
            let init = self.parse_expr();
            return Stmt::Let {
                span: word_span.to(init.span()),
                pattern,
                ty,
                init,
            };
        }

        // `name = value`. One token of lookahead is enough, and `==` is a
        // different token so there is nothing to disambiguate.
        if matches!(self.kind(), TokenKind::Ident(_)) && self.nth_kind(1) == &TokenKind::Eq {
            let target = self
                .expect_ident("an assignment")
                .expect("guarded by the lookahead above");
            self.bump();
            let value = self.parse_expr();
            return Stmt::Assign {
                span: target.span.to(value.span()),
                target,
                value,
            };
        }

        Stmt::Expr(self.parse_expr())
    }

    // -- expressions -------------------------------------------------------

    fn parse_expr(&mut self) -> Expr {
        self.parse_expr_bp(0)
    }

    /// Parses an expression in a position where `{` starts a block, such as an
    /// `if` condition or a contract obligation.
    fn parse_expr_no_struct(&mut self) -> Expr {
        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Deny);
        let expr = self.parse_expr();
        self.struct_lit = saved;
        expr
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_unary();

        // `0..10`. Two dots in a row are never anything else in this grammar,
        // because a field access has a name between them, so the shape can be
        // read here rather than left to fall apart further down.
        if self.at(&TokenKind::Dot)
            && self.nth_kind(1) == &TokenKind::Dot
            && !self.continues_a_new_line()
        {
            lhs = self.no_such_range(lhs);
        }

        // `n as String`. `as` stays an ordinary name, so this is the shape and
        // not the word: a value followed by a name followed by a name, all on
        // one line, which no expression could ever have been.
        if matches!(self.kind(), TokenKind::Ident(word) if word == "as")
            && matches!(self.nth_kind(1), TokenKind::Ident(_))
            && !self.continues_a_new_line()
            && !self.nth(1).starts_line
        {
            lhs = self.no_such_cast(lhs);
        }

        while let Some((op, bp)) = binary_op(self.kind()) {
            if bp < min_bp || self.continues_a_new_line() {
                break;
            }
            let op_span = self.bump().span;
            let rhs = self.parse_expr_bp(bp + 1);
            lhs = Expr::Binary {
                span: lhs.span().to(rhs.span()),
                op,
                op_span,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }

        lhs
    }

    /// Reports `a..b` and takes the whole thing with it.
    ///
    /// Leaving the dots where they were cost the rest of the file. The `for`
    /// they were usually written in went looking for its block, found a dot,
    /// and from there the body became a struct literal, its call became a
    /// missing brace, and the next declaration was reported as not being one:
    /// six diagnostics, none of them the mistake. Reading the bound and
    /// throwing it away means the block after it is still a block.
    fn no_such_range(&mut self, lhs: Expr) -> Expr {
        let mut dots = self.span().to(self.nth(1).span);

        // `..=` is the same mistake with one more character in it.
        let inclusive = self.nth_kind(2) == &TokenKind::Eq;
        if inclusive {
            dots = dots.to(self.nth(2).span);
        }

        self.emit(
            Diagnostic::error(
                codes::NO_RANGE,
                self.file,
                dots,
                "there is no range in this language",
            )
            .with_primary_label("no such operator")
            .with_note(
                "a `for` walks a list that already exists, which is what lets it declare \
                 nothing and still stop, so there is one thing to walk and a range would be \
                 a second",
            )
            .with_note(
                "`repeat(value, count)` makes the list to walk, and `for item at i in ...` \
                 binds the position, which is where a count usually came from",
            ),
        );

        self.bump();
        self.bump();
        if inclusive {
            self.bump();
        }

        let rhs = self.parse_unary();
        Expr::Error(lhs.span().to(rhs.span()))
    }

    /// Reports `x as T` and reads the type it was given.
    ///
    /// The type is parsed rather than skipped, so `xs as List<Int>` is one
    /// mistake and not a comparison that never closes. What comes back is an
    /// error node, because there is no cast to build: this language converts
    /// by calling something, and the call says in its return type whether it
    /// can fail, which is exactly what a cast is for not saying.
    fn no_such_cast(&mut self, lhs: Expr) -> Expr {
        let word_span = self.span();
        self.bump();
        let ty = self.parse_type();

        // Only a plain name has a conversion to point at. `List<Int>` and
        // `Fn(Int) -> Int` are the shapes nothing converts to anyway.
        let target = match &ty {
            Type::Named { name, args, .. } if args.is_empty() => Some(name.name.as_str()),
            _ => None,
        };

        let mut diagnostic = Diagnostic::error(
            codes::NO_CAST,
            self.file,
            word_span.to(ty.span()),
            "there is no cast in this language",
        )
        .with_primary_label("no such operator")
        .with_note(
            "a conversion is a call, and a call says in its return type whether it can fail, \
             which is the thing a cast is for not saying",
        );

        // The tail is everything from the end of the value to the end of the
        // type, so wrapping the value is an insertion in front of it and a
        // closing parenthesis in place of the rest.
        let tail = Span::new(lhs.span().end, ty.span().end);
        diagnostic = match target {
            Some("String") => diagnostic
                .with_note("a number is written out with `to_string(n)`")
                .with_edits(
                    "call `to_string`",
                    vec![
                        SuggestedEdit {
                            span: Span::at(lhs.span().start),
                            replacement: "to_string(".to_string(),
                        },
                        SuggestedEdit {
                            span: tail,
                            replacement: ")".to_string(),
                        },
                    ],
                    Applicability::MachineApplicable,
                ),
            Some("Int") => diagnostic
                .with_note(
                    "text is read as a number with `to_int(s)`, which gives a `Result` \
                     because not every string is one",
                )
                .with_edits(
                    "call `to_int`",
                    vec![
                        SuggestedEdit {
                            span: Span::at(lhs.span().start),
                            replacement: "to_int(".to_string(),
                        },
                        SuggestedEdit {
                            span: tail,
                            replacement: ")".to_string(),
                        },
                    ],
                    // Unlike `to_string` this one can fail, so what it gives
                    // back is a `Result` and what to do with that is the
                    // reader's decision rather than a rewrite anybody can make
                    // without them.
                    Applicability::MaybeIncorrect,
                ),
            _ => diagnostic.with_note(
                "`to_string` and `to_int` are the conversions the prelude has, and anything \
                 else is an ordinary function with a name of its own",
            ),
        };
        self.emit(diagnostic);

        Expr::Error(lhs.span().to(ty.span()))
    }

    fn parse_unary(&mut self) -> Expr {
        let op = match self.kind() {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Bang => Some(UnaryOp::Not),
            _ => None,
        };

        match op {
            Some(op) => {
                let op_span = self.bump().span;
                let operand = self.parse_unary();
                Expr::Unary {
                    span: op_span.to(operand.span()),
                    op,
                    op_span,
                    operand: Box::new(operand),
                }
            }
            None => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();

        loop {
            if self.continues_a_new_line() {
                break;
            }
            if self.at(&TokenKind::Dot) && matches!(self.nth_kind(1), TokenKind::Ident(_)) {
                self.bump();
                let name = self
                    .expect_ident("a field access")
                    .unwrap_or_else(|| Ident::new("", self.span()));
                expr = Expr::Field {
                    span: expr.span().to(name.span),
                    receiver: Box::new(expr),
                    name,
                };
            } else if self.at(&TokenKind::LParen) {
                let (args, end) = self.parse_call_args();
                expr = Expr::Call {
                    span: expr.span().to(end),
                    callee: Box::new(expr),
                    args,
                };
            } else if self.at(&TokenKind::Question) {
                let end = self.bump().span;
                expr = Expr::Try {
                    span: expr.span().to(end),
                    operand: Box::new(expr),
                };
            } else if self.at(&TokenKind::LBrace) && self.struct_lit_allowed() {
                let (fields, end) = self.parse_struct_lit_fields();
                expr = Expr::StructLit {
                    span: expr.span().to(end),
                    path: Box::new(expr),
                    fields,
                };
            } else {
                break;
            }
        }

        expr
    }

    /// Whether the token in hand is on a line of its own.
    ///
    /// Statements are separated by nothing, so what ends one is the next token
    /// not being able to continue the expression before it. That works for
    /// almost every token and fails silently for the two that can both start
    /// an expression and continue one. `(` reads as a call, so a statement
    /// beginning with a parenthesis attached itself to the line above. `-`
    /// reads as a subtraction, so `let a = 1` followed by `-2` became
    /// `let a = 1 - 2` and the second line was gone, with nothing to say so.
    ///
    /// A line break is what a reader uses to tell those apart, so it is what
    /// this uses. The rule is the same everywhere rather than switched off
    /// inside brackets, because "an expression ends at the end of a line" is
    /// one sentence and the version with an exception in it is three. `deed fmt`
    /// never breaks a binary expression or puts a call's parenthesis on a line
    /// of its own, so nothing canonical changes shape.
    fn continues_a_new_line(&self) -> bool {
        self.peek().starts_line
    }

    fn struct_lit_allowed(&self) -> bool {
        match self.struct_lit {
            StructLit::Allow => true,
            StructLit::Deny => false,
            StructLit::RequireColon => {
                if matches!(self.nth_kind(1), TokenKind::Ident(_))
                    && matches!(self.nth_kind(2), TokenKind::Colon)
                {
                    return true;
                }

                // `Empty { }`. A record is allowed to have no fields, so a
                // literal is allowed to have no fields either, and the rule
                // above cannot see one because there is no name to look at.
                //
                // `{ } {` decides it. An empty block is the value `()`, and
                // nothing in this language puts a block straight after a
                // value, so the only reading left is a literal with the block
                // that follows the handler list behind it. `with H { }` on its
                // own still reads as a handler and an empty body, which is
                // what it looks like.
                matches!(self.nth_kind(1), TokenKind::RBrace)
                    && matches!(self.nth_kind(2), TokenKind::LBrace)
            }
        }
    }

    fn parse_call_args(&mut self) -> (Vec<Expr>, Span) {
        self.bump();
        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
        let mut args = Vec::new();

        while !self.at(&TokenKind::RParen) && !self.at_eof() {
            let before = self.pos;
            args.push(self.parse_expr());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RParen, "an argument list");
        self.struct_lit = saved;
        (args, end)
    }

    fn parse_struct_lit_fields(&mut self) -> (Vec<FieldInit>, Span) {
        self.bump();
        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
        let mut fields = Vec::new();

        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            let Some(name) = self.expect_ident("a struct literal") else {
                break;
            };
            let value = if self.eat(&TokenKind::Colon) {
                Some(self.parse_expr())
            } else {
                None
            };
            fields.push(FieldInit {
                span: name
                    .span
                    .to(value.as_ref().map(Expr::span).unwrap_or(name.span)),
                name,
                value,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "a struct literal");
        self.struct_lit = saved;
        (fields, end)
    }

    /// `[1, 2, 3]`, with the `[` still to be read.
    ///
    /// A struct literal is allowed inside, the same as inside an argument
    /// list. The restriction exists to keep `if x { }` from reading as a
    /// literal, and a bracket has already committed to being an expression.
    fn parse_list(&mut self) -> Expr {
        let start = self.bump().span;
        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
        let mut elements = Vec::new();

        while !self.at(&TokenKind::RBracket) && !self.at_eof() {
            let before = self.pos;
            elements.push(self.parse_expr());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBracket, "a list literal");
        self.struct_lit = saved;
        Expr::List {
            elements,
            span: start.to(end),
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let span = self.span();

        match self.kind().clone() {
            TokenKind::Int(value) => {
                self.bump();
                Expr::Int { value, span }
            }
            TokenKind::Str(value) => {
                self.bump();
                Expr::Str { value, span }
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                Expr::Bool { value: true, span }
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                Expr::Bool { value: false, span }
            }
            TokenKind::Ident(name) => {
                self.bump();
                Expr::Ident(Ident::new(name, span))
            }
            TokenKind::LParen => {
                self.bump();
                if self.at(&TokenKind::RParen) {
                    let end = self.bump().span;
                    return Expr::Unit(span.to(end));
                }
                let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
                let inner = self.parse_expr();
                self.struct_lit = saved;
                self.expect(TokenKind::RParen, "a parenthesised expression");
                inner
            }
            TokenKind::LBracket => self.parse_list(),
            TokenKind::LBrace => Expr::Block(self.parse_block()),
            TokenKind::Pipe | TokenKind::PipePipe => self.parse_closure(),
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::Match) => self.parse_match(),
            TokenKind::Keyword(Keyword::For) => self.parse_for(),
            TokenKind::Keyword(Keyword::With) => self.parse_with(),
            TokenKind::Keyword(Keyword::Old) => {
                self.bump();
                self.expect(TokenKind::LParen, "`old`");
                let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
                let inner = self.parse_expr();
                self.struct_lit = saved;
                let end = self.span();
                self.expect(TokenKind::RParen, "`old`");
                Expr::Old {
                    expr: Box::new(inner),
                    span: span.to(end),
                }
            }
            TokenKind::Keyword(Keyword::Unchanged) => {
                self.bump();
                self.expect(TokenKind::LParen, "`unchanged`");
                let effect = self.parse_effect_ref();
                let end = self.span();
                self.expect(TokenKind::RParen, "`unchanged`");
                Expr::Unchanged {
                    effect,
                    span: span.to(end),
                }
            }
            other => {
                let found = other.describe();
                let mut diagnostic = Diagnostic::error(
                    codes::UNEXPECTED_TOKEN,
                    self.file,
                    span,
                    format!("expected an expression, found {found}"),
                )
                .with_primary_label("expected an expression");

                // An expression ends at the end of a line, so a line that
                // starts with an operator is a new statement rather than more
                // of the one above. Every language that lets you break a long
                // sum over two lines makes this a natural thing to write, and
                // without the note the error says what happened and none of
                // why.
                let carried_over = binary_op(&other).is_some() && self.peek().starts_line;
                if carried_over {
                    diagnostic = diagnostic.with_note(
                        "an expression ends at the end of a line, so this starts a new statement; \
                         leave the operator on the line above to carry it over",
                    );
                }

                self.emit(diagnostic);
                // Consume it so the caller always makes progress.
                if !self.at_eof() {
                    self.bump();
                }

                // What follows the operator was meant to be its right hand
                // side, so it is part of the same mistake. Taking it here
                // keeps it from becoming a statement of its own and drawing a
                // second complaint from a later pass.
                if carried_over && !self.at_eof() {
                    let rest = self.parse_expr_bp(0);
                    return Expr::Error(span.to(rest.span()));
                }

                Expr::Error(span)
            }
        }
    }

    fn parse_closure(&mut self) -> Expr {
        let start = self.span();
        let mut params = Vec::new();

        if self.at(&TokenKind::PipePipe) {
            self.bump();
        } else {
            self.bump();
            while !self.at(&TokenKind::Pipe) && !self.at_eof() {
                let before = self.pos;
                let Some(name) = self.expect_ident("a closure parameter") else {
                    break;
                };
                let ty = if self.eat(&TokenKind::Colon) {
                    Some(self.parse_type())
                } else {
                    // Nothing can infer this. A `let f = |x| ..` has no
                    // expected type to push down, and Deed does not do global
                    // inference, so leaving it out means the body is checked
                    // against nothing.
                    self.emit(
                        Diagnostic::error(
                            codes::MISSING_PARAMETER_TYPE,
                            self.file,
                            name.span,
                            format!("`{}` has no type", name.name),
                        )
                        .with_primary_label("a closure parameter needs a type")
                        .with_note(
                            "a parameter with no type is the unknown type, and the unknown type agrees with everything, so the body would not be checked",
                        ),
                    );
                    None
                };
                params.push(Param {
                    span: name
                        .span
                        .to(ty.as_ref().map(Type::span).unwrap_or(name.span)),
                    name,
                    ty,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.pos == before {
                    break;
                }
            }
            self.expect(TokenKind::Pipe, "a closure parameter list");
        }

        let body = self.parse_expr();
        Expr::Closure {
            span: start.to(body.span()),
            params,
            body: Box::new(body),
        }
    }

    /// `for n in numbers with sum = 0 { ... }`, with `for` still to be read.
    ///
    /// `for n at i in numbers` binds where in the list the element was. `at`
    /// is a name everywhere else in the language, and it is the name of the
    /// prelude function that indexes a list, so it stays one: the only thing
    /// that can follow a `for` binder is `at` or `in`, so there is nothing for
    /// it to be confused with here and nothing to reserve.
    ///
    /// The iterable and the initial value are parsed with struct literals
    /// held back, for the same reason an `if` condition is: the brace after
    /// them opens the body, and a name followed by one would otherwise read as
    /// a literal.
    fn parse_for(&mut self) -> Expr {
        let start = self.bump().span;

        let binder = self
            .expect_ident("a `for` loop")
            .unwrap_or_else(|| Ident::new("", self.span()));
        let index = if self.eat_named("at") {
            Some(
                self.expect_ident("a `for` index")
                    .unwrap_or_else(|| Ident::new("", self.span())),
            )
        } else {
            None
        };
        self.expect(TokenKind::Keyword(Keyword::In), "a `for` loop");

        let saved = std::mem::replace(&mut self.struct_lit, StructLit::RequireColon);
        let iterable = self.parse_expr();

        let accumulator = if self.at_kw(Keyword::With) {
            let with = self.bump().span;
            let name = self
                .expect_ident("a `for` accumulator")
                .unwrap_or_else(|| Ident::new("", self.span()));
            self.expect(TokenKind::Eq, "a `for` accumulator");
            let init = self.parse_expr();
            Some(Accumulator {
                span: with.to(init.span()),
                name,
                init: Box::new(init),
            })
        } else {
            None
        };

        // `while` is a name everywhere else, and the only thing that can come
        // between an accumulator and the body is this, so there is nothing for
        // it to be confused with and nothing to reserve. Same reasoning that
        // took `state` back out of the keyword list and kept `at` out of it.
        let keep = if self.eat_named("while") {
            Some(Box::new(self.parse_expr()))
        } else {
            None
        };
        self.struct_lit = saved;

        let body = self.parse_block();
        Expr::For {
            span: start.to(body.span),
            binder,
            index,
            iterable: Box::new(iterable),
            accumulator,
            keep,
            body,
        }
    }

    fn parse_if(&mut self) -> Expr {
        let start = self.bump().span;
        let condition = self.parse_expr_no_struct();
        let then_branch = self.parse_block();
        let else_branch = if self.eat_kw(Keyword::Else) {
            if self.at_kw(Keyword::If) {
                Some(Box::new(self.parse_if()))
            } else {
                Some(Box::new(Expr::Block(self.parse_block())))
            }
        } else {
            None
        };

        let end = else_branch
            .as_ref()
            .map(|e| e.span())
            .unwrap_or(then_branch.span);
        Expr::If {
            condition: Box::new(condition),
            then_branch,
            else_branch,
            span: start.to(end),
        }
    }

    fn parse_match(&mut self) -> Expr {
        let start = self.bump().span;
        let scrutinee = self.parse_expr_no_struct();
        self.expect(TokenKind::LBrace, "a match expression");

        let saved = std::mem::replace(&mut self.struct_lit, StructLit::Allow);
        let mut arms = Vec::new();

        while !self.at(&TokenKind::RBrace) && !self.at_eof() {
            let before = self.pos;
            let pattern = self.parse_arm_pattern();
            self.expect(TokenKind::FatArrow, "a match arm");
            let body = self.parse_expr();
            arms.push(MatchArm {
                span: pattern.span().to(body.span()),
                pattern,
                body,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }

        let end = self.span();
        self.expect(TokenKind::RBrace, "a match expression");
        self.struct_lit = saved;

        Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            span: start.to(end),
        }
    }

    fn parse_with(&mut self) -> Expr {
        let start = self.bump().span;

        let saved = std::mem::replace(&mut self.struct_lit, StructLit::RequireColon);
        let mut handlers = Vec::new();
        loop {
            if self.at(&TokenKind::LBrace) || self.at_eof() {
                break;
            }
            let before = self.pos;
            handlers.push(self.parse_expr());
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            if self.pos == before {
                break;
            }
        }
        self.struct_lit = saved;

        let body = self.parse_block();
        Expr::With {
            span: start.to(body.span),
            handlers,
            body,
        }
    }

    // -- patterns ----------------------------------------------------------

    /// One arm's pattern, which may name more than one variant.
    ///
    /// Only here. A `let` and a parameter take a single pattern, because the
    /// thing that makes alternatives cheap is that they bind nothing, and a
    /// binding form that binds nothing is a form with no reason to exist.
    ///
    /// There is nothing to disambiguate against. A closure also starts with
    /// `|`, and a closure is an expression, so the only `|` that can follow a
    /// pattern in an arm is this one.
    fn parse_arm_pattern(&mut self) -> Pattern {
        let first = self.parse_pattern();
        if !self.at(&TokenKind::Pipe) {
            return first;
        }

        let start = first.span();
        let mut alternatives = vec![first];
        while self.eat(&TokenKind::Pipe) {
            let before = self.pos;
            alternatives.push(self.parse_pattern());
            if self.pos == before {
                break;
            }
        }

        let span = start.to(alternatives.last().map_or(start, Pattern::span));
        Pattern::OneOf { alternatives, span }
    }

    fn parse_pattern(&mut self) -> Pattern {
        let span = self.span();

        match self.kind().clone() {
            TokenKind::Underscore => {
                self.bump();
                Pattern::Wildcard(span)
            }
            TokenKind::Int(value) => {
                self.bump();
                Pattern::Int { value, span }
            }
            TokenKind::Str(value) => {
                self.bump();
                Pattern::Str { value, span }
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                Pattern::Bool { value: true, span }
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                Pattern::Bool { value: false, span }
            }
            TokenKind::Ident(_) => self.parse_path_pattern(),
            other => {
                let found = other.describe();
                self.emit(
                    Diagnostic::error(
                        codes::UNEXPECTED_TOKEN,
                        self.file,
                        span,
                        format!("expected a pattern, found {found}"),
                    )
                    .with_primary_label("expected a pattern"),
                );
                if !self.at_eof() {
                    self.bump();
                }
                Pattern::Error(span)
            }
        }
    }

    fn parse_path_pattern(&mut self) -> Pattern {
        let mut segments = Vec::new();
        let start = self.span();

        while let Some(segment) = self.expect_ident("a pattern") {
            segments.push(segment);
            if !(self.at(&TokenKind::Dot) && matches!(self.nth_kind(1), TokenKind::Ident(_))) {
                break;
            }
            self.bump();
        }

        let path_end = segments.last().map(|s| s.span).unwrap_or(start);

        if self.at(&TokenKind::LParen) {
            self.bump();
            let mut elements = Vec::new();
            while !self.at(&TokenKind::RParen) && !self.at_eof() {
                let before = self.pos;
                elements.push(self.parse_pattern());
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.pos == before {
                    break;
                }
            }
            let end = self.span();
            self.expect(TokenKind::RParen, "a pattern");
            return Pattern::Tuple {
                path: segments,
                elements,
                span: start.to(end),
            };
        }

        if self.at(&TokenKind::LBrace) {
            self.bump();
            let mut fields = Vec::new();
            while !self.at(&TokenKind::RBrace) && !self.at_eof() {
                let before = self.pos;
                let Some(name) = self.expect_ident("a pattern field") else {
                    break;
                };
                let pattern = if self.eat(&TokenKind::Colon) {
                    Some(self.parse_pattern())
                } else {
                    None
                };
                fields.push(PatternField {
                    span: name
                        .span
                        .to(pattern.as_ref().map(Pattern::span).unwrap_or(name.span)),
                    name,
                    pattern,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.pos == before {
                    break;
                }
            }
            let end = self.span();
            self.expect(TokenKind::RBrace, "a pattern");
            return Pattern::Record {
                path: segments,
                fields,
                span: start.to(end),
            };
        }

        Pattern::Path {
            segments,
            span: start.to(path_end),
        }
    }
}

fn binary_op(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    let pair = match kind {
        TokenKind::PipePipe => (BinaryOp::Or, 1),
        TokenKind::AmpAmp => (BinaryOp::And, 2),
        TokenKind::EqEq => (BinaryOp::Eq, 3),
        TokenKind::BangEq => (BinaryOp::Ne, 3),
        TokenKind::Lt => (BinaryOp::Lt, 3),
        TokenKind::Le => (BinaryOp::Le, 3),
        TokenKind::Gt => (BinaryOp::Gt, 3),
        TokenKind::Ge => (BinaryOp::Ge, 3),
        TokenKind::Plus => (BinaryOp::Add, 4),
        TokenKind::Minus => (BinaryOp::Sub, 4),
        TokenKind::Star => (BinaryOp::Mul, 5),
        TokenKind::Slash => (BinaryOp::Div, 5),
        TokenKind::Percent => (BinaryOp::Rem, 5),
        _ => return None,
    };
    Some(pair)
}
