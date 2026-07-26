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

use vow_ast::{
    Accumulator, BinaryOp, Block, ChoiceDecl, Contract, EffectDecl, EffectRef, Ensures, Expr,
    FieldDecl, FieldInit, FnDecl, FnSig, HandlerDecl, Ident, Item, MatchArm, Module, ModulePath,
    Outcome, Param, Pattern, PatternField, RecordDecl, Stmt, TestDecl, Type, TypeAlias, UnaryOp,
    Use, Variant,
};
use vow_diagnostics::{Applicability, Diagnostic, FileId, Span};
use vow_lexer::{Keyword, Token, TokenKind};

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
    /// Handler position in `with`. A brace is a struct literal only if it is
    /// followed by `name:`, which is enough to tell `InMemoryLedger { a: 1 }`
    /// from the block that follows the handler list.
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

        let mut uses = Vec::new();
        while self.at_kw(Keyword::Use) {
            let before = self.pos;
            self.bump();
            match self.parse_use() {
                Some(item) => uses.push(item),
                None => self.synchronize_item(),
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
                uses,
                items,
                span: start.to(end),
            },
            diagnostics: self.diagnostics,
        }
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
            self.emit(
                Diagnostic::error(
                    codes::EXPECTED_DECLARATION,
                    self.file,
                    span,
                    format!("expected a declaration, found {found}"),
                )
                .with_primary_label("not the start of a declaration")
                .with_note(
                    "a file contains `type`, `record`, `choice`, `effect`, `handler`, `fn` and `test` declarations",
                ),
            );
            return None;
        };

        match kw {
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

    /// `type Positive = Int where value > 0`
    fn parse_type_alias(&mut self) -> Option<TypeAlias> {
        let start = self.bump().span;
        let name = self.expect_ident("a type alias")?;
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
            ty,
            refinement,
            span: start.to(end),
        })
    }

    fn parse_record(&mut self) -> Option<RecordDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("a record declaration")?;
        let (fields, end) = self.parse_field_block("a record declaration")?;
        Some(RecordDecl {
            name,
            fields,
            span: start.to(end),
        })
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

    fn parse_choice(&mut self) -> Option<ChoiceDecl> {
        let start = self.bump().span;
        let name = self.expect_ident("a choice declaration")?;
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
            if self.eat_kw(Keyword::State) {
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
        let outcome = match self.kind() {
            TokenKind::Ident(name) if name == "ok" => Some(Outcome::Ok),
            TokenKind::Ident(name) if name == "err" => Some(Outcome::Err),
            _ => None,
        };

        let outcome = match outcome {
            Some(outcome) => {
                self.bump();
                outcome
            }
            None => {
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
                // Consume it anyway. Whatever it was, it stood where the
                // outcome belongs, and leaving it would derail the `=>` that
                // follows into a second diagnostic about the same mistake.
                if !self.at_eof() && !self.at(&TokenKind::FatArrow) {
                    self.bump();
                }
                Outcome::Ok
            }
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
            // There is no shift operator in Vow, so `Map<K, Vec<V>>` closes with
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
            let condition = self.parse_expr();
            return Stmt::Assert {
                span: start.to(condition.span()),
                condition,
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

        while let Some((op, bp)) = binary_op(self.kind()) {
            if bp < min_bp {
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

    fn struct_lit_allowed(&self) -> bool {
        match self.struct_lit {
            StructLit::Allow => true,
            StructLit::Deny => false,
            StructLit::RequireColon => {
                matches!(self.nth_kind(1), TokenKind::Ident(_))
                    && matches!(self.nth_kind(2), TokenKind::Colon)
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
                self.emit(
                    Diagnostic::error(
                        codes::UNEXPECTED_TOKEN,
                        self.file,
                        span,
                        format!("expected an expression, found {found}"),
                    )
                    .with_primary_label("expected an expression"),
                );
                // Consume it so the caller always makes progress.
                if !self.at_eof() {
                    self.bump();
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
                    // expected type to push down, and Vow does not do global
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
    /// The iterable and the initial value are parsed with struct literals
    /// held back, for the same reason an `if` condition is: the brace after
    /// them opens the body, and a name followed by one would otherwise read as
    /// a literal.
    fn parse_for(&mut self) -> Expr {
        let start = self.bump().span;

        let binder = self
            .expect_ident("a `for` loop")
            .unwrap_or_else(|| Ident::new("", self.span()));
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
        self.struct_lit = saved;

        let body = self.parse_block();
        Expr::For {
            span: start.to(body.span),
            binder,
            iterable: Box::new(iterable),
            accumulator,
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
            let pattern = self.parse_pattern();
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
