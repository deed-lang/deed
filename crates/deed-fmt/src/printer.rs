//! Turning a tree back into text.
//!
//! The shape of this file is one method per node kind and a small set of
//! layout decisions applied everywhere. The decisions are written down next to
//! the code that makes them, because a formatter is nothing but a pile of
//! decisions and one that does not say why is impossible to argue with.

use std::fmt::Write;

use deed_ast::{
    BinaryOp, Block, ChoiceDecl, Contract, DeprecateDecl, EffectDecl, EffectRef, Expr, FieldDecl,
    FieldInit, FnDecl, FnSig, HandlerDecl, Ident, Item, MatchArm, Module, Outcome, Param, Pattern,
    PatternField, RecordDecl, Stmt, TestDecl, Type, TypeAlias, UnaryOp, Use, Variant,
};
use deed_lexer::Trivia;

/// The column a line tries to stay inside.
///
/// Not a hard limit: a single long name or a deeply nested expression can go
/// past it, because breaking those produces something worse than a long line.
const WIDTH: usize = 90;

const INDENT: &str = "    ";

pub fn print(source: &str, module: &Module, trivia: &[Trivia]) -> String {
    let mut printer = Printer {
        source,
        trivia,
        next_trivia: 0,
        out: String::new(),
        indent: 0,
        pending: 0,
        last_end: 0,
    };
    printer.module(module);
    printer.out
}

struct Printer<'a> {
    source: &'a str,
    trivia: &'a [Trivia],
    /// Index of the first comment not yet emitted.
    next_trivia: usize,
    out: String,
    indent: usize,
    /// Columns already committed on the current line by an enclosing prefix
    /// such as `return ` or `let x = `.
    ///
    /// Rendering is bottom up, so an inner expression measuring only itself
    /// decides it fits and then overshoots once the prefix is prepended. This
    /// is the cheap version of threading a width budget down the tree, and it
    /// covers the case that actually shows up.
    pending: usize,
    /// Where the last printed node ended in the source, so a comment on the
    /// same line can be recognised as trailing rather than leading.
    last_end: u32,
}

impl Printer<'_> {
    // -- output primitives -------------------------------------------------

    fn push(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn newline(&mut self) {
        self.out.push('\n');
    }

    /// Starts a line at the current indent.
    fn line_start(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str(INDENT);
        }
    }

    fn blank_line(&mut self) {
        if !self.out.ends_with("\n\n") && !self.out.is_empty() {
            self.newline();
        }
    }

    // -- comments ----------------------------------------------------------

    /// Emits every comment that appears before `offset` in the source.
    ///
    /// A comment on the same line as the thing before it stays there. Anything
    /// else goes on its own line at the current indent, and at most one blank
    /// line survives between comments, which is the compromise every formatter
    /// converges on: collapsing all of them is unreadable, keeping all of them
    /// is not canonical.
    fn comments_before(&mut self, offset: u32) {
        let mut buffer = std::mem::take(&mut self.out);
        self.comments_into(&mut buffer, offset);
        self.out = buffer;
    }

    /// The same, into whatever buffer is being built.
    ///
    /// Blocks nested inside an expression are rendered into a local string
    /// rather than straight to the output, and a comment inside one of those
    /// has to land there or it surfaces somewhere else entirely, which is what
    /// happened to a comment in `counter.deed` the first time this ran.
    fn comments_into(&mut self, buffer: &mut String, offset: u32) {
        while self.next_trivia < self.trivia.len() {
            let comment = &self.trivia[self.next_trivia];
            if comment.span.start >= offset {
                break;
            }
            self.next_trivia += 1;

            let text =
                self.source[comment.span.start as usize..comment.span.end as usize].trim_end();
            let trailing = self.on_same_line_as_previous(comment.span.start);

            if trailing {
                // ` // like this`, kept where it was written.
                while buffer.ends_with('\n') {
                    buffer.pop();
                }
                buffer.push(' ');
            } else {
                if self.blank_line_before(comment.span.start) {
                    blank_line(buffer);
                }
                buffer.push_str(&INDENT.repeat(self.indent));
            }

            let indent = INDENT.repeat(self.indent);
            let mut lines = text.lines();
            if let Some(first) = lines.next() {
                buffer.push_str(first.trim_end());
            }
            for line in lines {
                buffer.push('\n');
                buffer.push_str(&indent);
                buffer.push_str(line.trim());
            }
            buffer.push('\n');

            self.last_end = comment.span.end;
        }
    }

    /// Everything left over, which is whatever trails the last item.
    fn remaining_comments(&mut self) {
        self.comments_before(u32::MAX);
    }

    fn on_same_line_as_previous(&self, start: u32) -> bool {
        let from = self.last_end as usize;
        let to = start as usize;
        from > 0 && to > from && !self.source[from..to].contains('\n')
    }

    /// Whether the source had a blank line before `start`.
    fn blank_line_before(&self, start: u32) -> bool {
        let from = self.last_end as usize;
        let to = start as usize;
        to > from && self.source[from..to].matches('\n').count() >= 2
    }

    /// Keeps a blank line the source had, and no more than one.
    fn maybe_blank_before(&mut self, start: u32) {
        if self.blank_line_before(start) {
            self.blank_line();
        }
    }

    // -- module ------------------------------------------------------------

    fn module(&mut self, module: &Module) {
        if let Some(name) = &module.name {
            self.comments_before(name.span.start);
            self.maybe_blank_before(name.span.start);
            self.line_start();
            self.push("module ");
            self.push(&name.to_string_path());
            if let Some(edition) = &module.edition {
                self.push(" edition ");
                self.push(&edition.year.to_string());
            }
            self.newline();
            self.last_end = module
                .edition
                .as_ref()
                .map_or(name.span.end, |e| e.span.end);
        }

        for use_decl in &module.uses {
            self.blank_line();
            self.comments_before(use_decl.span.start);
            self.use_decl(use_decl);
            self.last_end = use_decl.span.end;
        }

        // One blank line between items, always, and the comment above an item
        // belongs to the item rather than to the gap.
        for item in &module.items {
            self.blank_line();
            self.comments_before(item.span().start);
            // A comment with a blank line after it was not written about the
            // item below it, so the gap stays and the attachment does not.
            self.maybe_blank_before(item.span().start);
            self.item(item);
            self.last_end = item.span().end;
        }

        self.remaining_comments();

        // Exactly one trailing newline, whatever the input had.
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.newline();
        }
    }

    fn use_decl(&mut self, use_decl: &Use) {
        self.line_start();
        self.push("use ");
        self.push(&use_decl.path.to_string_path());
        self.push(".{");
        for (index, name) in use_decl.names.iter().enumerate() {
            if index > 0 {
                self.push(", ");
            }
            self.push(&name.name);
        }
        self.push("}");
        self.newline();
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Deprecate(decl) => self.deprecate(decl),
            Item::TypeAlias(decl) => self.type_alias(decl),
            Item::Record(decl) => self.record(decl),
            Item::Choice(decl) => self.choice(decl),
            Item::Effect(decl) => self.effect(decl),
            Item::Handler(decl) => self.handler(decl),
            Item::Function(decl) => self.function(decl),
            Item::Test(decl) => self.test(decl),
        }
    }

    fn deprecate(&mut self, decl: &DeprecateDecl) {
        self.line_start();
        self.push("deprecated ");
        self.push(&decl.old.name);
        self.push(" -> ");
        self.push(&decl.new.name);
        self.newline();
    }

    fn type_alias(&mut self, decl: &TypeAlias) {
        self.line_start();
        self.push("type ");
        self.push(&decl.name.name);
        self.push(&type_params(&decl.generics, &[]));
        self.push(" = ");
        let ty = self.ty(&decl.ty);
        self.push(&ty);
        if let Some(refinement) = &decl.refinement {
            self.push(" where ");
            let text = self.expr(refinement);
            self.push(&text);
        }
        self.newline();
    }

    fn record(&mut self, decl: &RecordDecl) {
        self.line_start();
        self.push("record ");
        self.push(&decl.name.name);
        self.push(&type_params(&decl.generics, &[]));
        self.fields_block(&decl.fields);
    }

    /// `{ ... }` with one field per line, or `{}` when there are none.
    fn fields_block(&mut self, fields: &[FieldDecl]) {
        if fields.is_empty() {
            self.push(" {}");
            self.newline();
            return;
        }

        self.push(" {");
        self.newline();
        self.indent += 1;
        for field in fields {
            self.comments_before(field.span.start);
            self.line_start();
            let text = self.field_decl(field);
            self.push(&text);
            self.push(",");
            self.newline();
            self.last_end = field.span.end;
        }
        self.indent -= 1;
        self.line_start();
        self.push("}");
        self.newline();
    }

    fn field_decl(&mut self, field: &FieldDecl) -> String {
        format!("{}: {}", field.name.name, self.ty(&field.ty))
    }

    fn choice(&mut self, decl: &ChoiceDecl) {
        self.line_start();
        self.push("choice ");
        self.push(&decl.name.name);
        self.push(&type_params(&decl.generics, &[]));

        if decl.variants.is_empty() {
            self.push(" {}");
            self.newline();
            return;
        }

        self.push(" {");
        self.newline();
        self.indent += 1;
        for variant in &decl.variants {
            self.comments_before(variant.span.start);
            self.line_start();
            let text = self.variant(variant);
            self.push(&text);
            self.push(",");
            self.newline();
            self.last_end = variant.span.end;
        }
        self.indent -= 1;
        self.line_start();
        self.push("}");
        self.newline();
    }

    /// A variant is written on one line even when it carries fields.
    ///
    /// A choice is a list of alternatives and the list is what a reader is
    /// scanning. Breaking one alternative across four lines buries it.
    fn variant(&mut self, variant: &Variant) -> String {
        let mut text = variant.name.name.clone();
        if let Some(fields) = &variant.fields {
            let inner: Vec<String> = fields
                .iter()
                .map(|field| format!("{}: {}", field.name.name, self.ty(&field.ty)))
                .collect();
            let _ = write!(text, " {{ {} }}", inner.join(", "));
        }
        text
    }

    fn effect(&mut self, decl: &EffectDecl) {
        self.line_start();
        self.push("effect ");
        self.push(&decl.name.name);
        self.push(" {");
        self.newline();
        self.indent += 1;
        for op in &decl.operations {
            self.comments_before(op.span.start);
            self.line_start();
            let text = self.signature_line(op);
            self.push(&text);
            self.newline();
            self.last_end = op.span.end;
        }
        self.indent -= 1;
        self.line_start();
        self.push("}");
        self.newline();
    }

    fn handler(&mut self, decl: &HandlerDecl) {
        self.line_start();
        self.push("handler ");
        self.push(&decl.name.name);
        self.push(" implements ");
        self.push(&decl.effect.name);
        self.push(" {");
        self.newline();
        self.indent += 1;

        for field in &decl.state {
            self.comments_before(field.span.start);
            self.line_start();
            self.push("state ");
            let text = self.field_decl(field);
            self.push(&text);
            self.newline();
            self.last_end = field.span.end;
        }

        for op in &decl.operations {
            self.blank_line();
            self.comments_before(op.span.start);
            self.function(op);
            self.last_end = op.span.end;
        }

        if let Some(finally) = &decl.finally {
            self.blank_line();
            self.comments_before(finally.span.start);
            self.line_start();
            self.push("finally ");
            self.block(finally);
            self.newline();
            self.last_end = finally.span.end;
        }

        self.indent -= 1;
        self.line_start();
        self.push("}");
        self.newline();
    }

    fn test(&mut self, decl: &TestDecl) {
        self.line_start();
        self.push("test ");
        self.push(&string_literal(&decl.name));
        self.push(" ");
        self.block(&decl.body);
        self.newline();
    }

    // -- functions ---------------------------------------------------------

    fn function(&mut self, decl: &FnDecl) {
        let before = self.out.len();
        self.signature(&decl.sig, &decl.contract);

        // A signature that stayed on one line keeps its brace on the end of
        // it. One that did not puts the brace back at the left margin, because
        // everything a signature wraps onto is indented past that margin and a
        // brace tucked on the end of an indented line reads as part of it. A
        // contract is the usual reason to wrap and used to be the only one
        // asked about, so a return type long enough to wrap got the other
        // answer to the same question.
        if self.out[before..].contains('\n') {
            self.newline();
            self.line_start();
        } else {
            self.push(" ");
        }

        self.block(&decl.body);
        self.newline();
    }

    /// `fn name(a: T, b: U) -> R` on one line when it fits.
    fn signature_line(&mut self, sig: &FnSig) -> String {
        let params: Vec<String> = sig.params.iter().map(|p| self.param(p)).collect();
        let mut text = format!(
            "fn {}{}({})",
            sig.name.name,
            type_params(&sig.generics, &sig.rows),
            params.join(", ")
        );
        if let Some(ret) = &sig.ret {
            let _ = write!(text, " -> {}", self.ty(ret));
        }
        text
    }

    fn param(&mut self, param: &Param) -> String {
        match &param.ty {
            Some(ty) => format!("{}: {}", param.name.name, self.ty(ty)),
            // Handler operations inherit their types from the effect.
            None => param.name.name.clone(),
        }
    }

    /// The signature and its contract, which together are the review surface.
    ///
    /// Three layouts, tried in order, so the choice never depends on anything
    /// but how wide the result is:
    ///
    /// 1. everything on one line
    /// 2. the return type on its own line
    /// 3. one parameter per line as well
    fn signature(&mut self, sig: &FnSig, contract: &Contract) {
        let one_line = self.signature_line(sig);
        let width = self.indent * INDENT.len() + one_line.len();

        if width <= WIDTH {
            self.line_start();
            self.push(&one_line);
        } else {
            let params: Vec<String> = sig.params.iter().map(|p| self.param(p)).collect();
            let head = format!(
                "fn {}{}({})",
                sig.name.name,
                type_params(&sig.generics, &sig.rows),
                params.join(", ")
            );
            let head_width = self.indent * INDENT.len() + head.len();

            if head_width <= WIDTH {
                self.line_start();
                self.push(&head);
            } else {
                self.line_start();
                self.push(&format!(
                    "fn {}{}(",
                    sig.name.name,
                    type_params(&sig.generics, &sig.rows)
                ));
                self.newline();
                self.indent += 1;
                for param in &params {
                    self.line_start();
                    self.push(param);
                    self.push(",");
                    self.newline();
                }
                self.indent -= 1;
                self.line_start();
                self.push(")");
            }

            if let Some(ret) = &sig.ret {
                self.newline();
                self.indent += 1;
                self.line_start();
                self.push(&format!("-> {}", self.ty(ret)));
                self.indent -= 1;
            }
        }

        self.contract(contract);
    }

    /// `where`, `uses` and `ensures`, each one clause per line.
    ///
    /// These are indented half a step relative to the body, which is not an
    /// accident: they belong to the signature, and lining them up with the
    /// body would read as though they were part of it.
    fn contract(&mut self, contract: &Contract) {
        if contract.is_empty() {
            return;
        }

        let inner = self.indent + 1;

        if !contract.requires.is_empty() {
            self.clause_header("where");
            for condition in &contract.requires {
                let text = self.expr(condition);
                self.clause_line(inner, &text);
            }
        }

        if !contract.uses.is_empty() {
            self.clause_header("uses");
            for effect in &contract.uses {
                let text = effect_ref(effect);
                self.clause_line(inner, &text);
            }
        }

        if !contract.ensures.is_empty() {
            self.clause_header("ensures");
            for obligation in &contract.ensures {
                // `ok` padded to the width of `err`, always, so the arrows line
                // up whether or not both outcomes appear.
                let outcome = match obligation.outcome {
                    Outcome::Ok => "ok ",
                    Outcome::Err => "err",
                };
                let condition = self.expr(&obligation.condition);
                self.clause_line(inner, &format!("{outcome} => {condition}"));
            }
        }
    }

    fn clause_header(&mut self, keyword: &str) {
        self.newline();
        for _ in 0..self.indent {
            self.push(INDENT);
        }
        self.push("  ");
        self.push(keyword);
    }

    fn clause_line(&mut self, indent: usize, text: &str) {
        self.newline();
        for _ in 0..indent {
            self.push(INDENT);
        }
        self.push(text);
        self.push(",");
    }

    // -- blocks and statements ---------------------------------------------

    fn block(&mut self, block: &Block) {
        let text = self.rendered_block(block);
        self.push(&text);
    }

    // -- expressions -------------------------------------------------------

    /// Renders an expression, using the current indent for anything it has to
    /// break across lines.
    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Int { value, .. } => value.to_string(),
            Expr::Str { value, .. } => string_literal(value),
            Expr::Bool { value, .. } => value.to_string(),
            Expr::Unit(_) => "()".to_string(),
            Expr::Ident(ident) => ident.name.clone(),

            Expr::Field { receiver, name, .. } => {
                format!("{}.{}", self.postfix_base(receiver), name.name)
            }

            Expr::Call { callee, args, .. } => {
                let callee = self.postfix_base(callee);
                if args.is_empty() {
                    return format!("{callee}()");
                }
                // One argument that is itself a list breaks on its own rather
                // than being wrapped in a broken call, so `err(Thing { .. })`
                // does not turn into three levels of punctuation.
                if args.len() == 1 {
                    let arg = self.with_prefix(callee.len() + 1, &args[0]);
                    return format!("{callee}({arg})");
                }
                let rendered: Vec<String> = args.iter().map(|arg| self.expr(arg)).collect();
                let one_line = format!("{callee}({})", rendered.join(", "));
                if self.fits(&one_line) {
                    return one_line;
                }
                self.broken(&format!("{callee}("), args, ")", |me, arg| me.expr(arg))
            }

            Expr::List { elements, .. } => {
                if elements.is_empty() {
                    return "[]".to_string();
                }
                let rendered: Vec<String> = elements.iter().map(|e| self.expr(e)).collect();
                let one_line = format!("[{}]", rendered.join(", "));
                if self.fits(&one_line) {
                    return one_line;
                }
                self.broken("[", elements, "]", |me, element| me.expr(element))
            }

            Expr::StructLit { path, fields, .. } => {
                let path = self.postfix_base(path);
                let rendered: Vec<String> =
                    fields.iter().map(|field| self.field_init(field)).collect();
                if rendered.is_empty() {
                    return format!("{path} {{}}");
                }
                let one_line = format!("{path} {{ {} }}", rendered.join(", "));
                if self.fits(&one_line) {
                    return one_line;
                }
                self.broken(&format!("{path} {{"), fields, "}", |me, field| {
                    me.field_init(field)
                })
            }

            Expr::Unary { op, operand, .. } => {
                let symbol = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "!",
                };
                let operand = match operand.as_ref() {
                    inner @ (Expr::Binary { .. } | Expr::Try { .. }) => {
                        format!("({})", self.expr(inner))
                    }
                    inner => self.expr(inner),
                };
                format!("{symbol}{operand}")
            }

            Expr::Binary { op, lhs, rhs, .. } => {
                let lhs = self.operand(lhs, *op, false);
                let rhs = self.operand(rhs, *op, true);
                format!("{lhs} {} {rhs}", op.as_str())
            }

            Expr::Try { operand, .. } => format!("{}?", self.postfix_base(operand)),

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let condition = self.expr(condition);
                let then_text = self.rendered_block(then_branch);
                let mut text = format!("if {condition} {then_text}");
                if let Some(otherwise) = else_branch {
                    let _ = write!(text, " else {}", self.expr(otherwise));
                }
                text
            }

            Expr::Match {
                scrutinee, arms, ..
            } => {
                let scrutinee = self.expr(scrutinee);
                self.match_expr(&scrutinee, arms)
            }

            Expr::For {
                binder,
                index,
                iterable,
                accumulator,
                keep,
                body,
                ..
            } => {
                let mut head = format!("for {}", binder.name);
                if let Some(index) = index {
                    let _ = write!(head, " at {}", index.name);
                }
                let _ = write!(head, " in {}", self.expr(iterable));
                if let Some(accumulator) = accumulator {
                    let _ = write!(
                        head,
                        " with {} = {}",
                        accumulator.name.name,
                        self.expr(&accumulator.init)
                    );
                }
                if let Some(keep) = keep {
                    let _ = write!(head, " while {}", self.expr(keep));
                }
                format!("{head} {}", self.rendered_block(body))
            }

            Expr::Block(block) => self.rendered_block(block),

            Expr::Closure { params, body, .. } => {
                let params: Vec<String> = params.iter().map(|p| self.param(p)).collect();
                format!("|{}| {}", params.join(", "), self.expr(body))
            }

            Expr::Old { expr, .. } => format!("old({})", self.expr(expr)),
            Expr::Unchanged { effect, .. } => format!("unchanged({})", effect_ref(effect)),

            Expr::With { handlers, body, .. } => {
                let rendered: Vec<String> = handlers.iter().map(|h| self.expr(h)).collect();
                let head = rendered.join(", ");
                let block = self.rendered_block(body);
                if head.contains('\n') {
                    // A handler that had to break already ends in a `}`, and a
                    // second `{` tacked onto that line is unreadable.
                    format!("with {head}\n{}{block}", INDENT.repeat(self.indent))
                } else {
                    format!("with {head} {block}")
                }
            }

            Expr::Error(_) => String::new(),
        }
    }

    fn field_init(&mut self, field: &FieldInit) -> String {
        match &field.value {
            Some(value) => format!("{}: {}", field.name.name, self.expr(value)),
            // `Receipt { from }`, which the parser already treats as shorthand.
            None => field.name.name.clone(),
        }
    }

    /// A child of a binary operator, parenthesized when the tree says so.
    ///
    /// The AST does not record where the source had parentheses, so they have
    /// to be reconstructed from precedence. Getting this wrong changes what the
    /// program means, which is the one thing a formatter must never do.
    fn operand(&mut self, expr: &Expr, parent: BinaryOp, is_right: bool) -> String {
        let text = self.expr(expr);
        let needs = match expr {
            Expr::Binary { op, .. } => {
                precedence(*op) < precedence(parent)
                    || (precedence(*op) == precedence(parent) && is_right)
            }
            Expr::If { .. } | Expr::Match { .. } | Expr::With { .. } | Expr::Closure { .. } => true,
            _ => false,
        };
        if needs { format!("({text})") } else { text }
    }

    /// The receiver of a `.`, a call, or a `?`, parenthesized when needed.
    fn postfix_base(&mut self, expr: &Expr) -> String {
        let text = self.expr(expr);
        let needs = matches!(
            expr,
            Expr::Binary { .. }
                | Expr::Unary { .. }
                | Expr::If { .. }
                | Expr::Match { .. }
                | Expr::With { .. }
                | Expr::Closure { .. }
        );
        if needs { format!("({text})") } else { text }
    }

    fn match_expr(&mut self, scrutinee: &str, arms: &[MatchArm]) -> String {
        let mut text = format!("match {scrutinee} {{\n");
        self.indent += 1;
        for arm in arms {
            self.comments_into(&mut text, arm.span.start);
            let pattern = self.pattern(&arm.pattern);
            let body = self.expr(&arm.body);
            text.push_str(&INDENT.repeat(self.indent));
            let _ = writeln!(text, "{pattern} => {body},");
            self.last_end = arm.span.end;
        }
        self.indent -= 1;
        text.push_str(&INDENT.repeat(self.indent));
        text.push('}');
        text
    }

    /// A block, rendered into a string so it can be nested inside anything.
    ///
    /// The whole body goes through here, including a function's, so there is
    /// one place that decides what a block looks like.
    fn rendered_block(&mut self, block: &Block) -> String {
        if block.stmts.is_empty()
            && block.tail.is_none()
            && !self.has_comments_before(block.span.end)
        {
            // An empty body still has to be written down, and `{}` says it in
            // one place rather than two lines of nothing.
            return "{}".to_string();
        }

        let mut text = String::from("{\n");
        self.indent += 1;
        // Whatever separated the header from the brace is behind us, so a gap
        // there is not a blank line inside the body.
        self.last_end = block.span.start;

        for stmt in &block.stmts {
            self.comments_into(&mut text, stmt.span().start);
            // A gap right after the opening brace is noise, not structure.
            if self.blank_line_before(stmt.span().start) && !text.ends_with("{\n") {
                blank_line(&mut text);
            }
            text.push_str(&INDENT.repeat(self.indent));
            text.push_str(&self.stmt_text(stmt));
            text.push('\n');
            self.last_end = stmt.span().end;
        }

        if let Some(tail) = &block.tail {
            self.comments_into(&mut text, tail.span().start);
            if self.blank_line_before(tail.span().start) && !text.ends_with("{\n") {
                blank_line(&mut text);
            }
            text.push_str(&INDENT.repeat(self.indent));
            text.push_str(&self.expr(tail));
            text.push('\n');
            self.last_end = tail.span().end;
        }

        // A comment as the last thing in a block belongs to the block, not to
        // whatever comes after the closing brace.
        self.comments_into(&mut text, block.span.end);

        self.indent -= 1;
        text.push_str(&INDENT.repeat(self.indent));
        text.push('}');
        text
    }

    fn has_comments_before(&self, offset: u32) -> bool {
        self.trivia
            .get(self.next_trivia)
            .is_some_and(|comment| comment.span.start < offset)
    }

    fn stmt_text(&mut self, stmt: &Stmt) -> String {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                let mut text = format!("let {}", self.pattern(pattern));
                if let Some(ty) = ty {
                    let _ = write!(text, ": {}", self.ty(ty));
                }
                let prefix = text.len() + 3;
                let _ = write!(text, " = {}", self.with_prefix(prefix, init));
                text
            }
            Stmt::Assign { target, value, .. } => {
                let prefix = target.name.len() + 3;
                format!("{} = {}", target.name, self.with_prefix(prefix, value))
            }
            Stmt::Return { value, .. } => match value {
                Some(value) => format!("return {}", self.with_prefix(7, value)),
                None => "return".to_string(),
            },
            Stmt::Abandon { .. } => "abandon".to_string(),
            Stmt::Assert { condition, .. } => {
                format!("assert {}", self.with_prefix(7, condition))
            }
            Stmt::Refuses { subject, .. } => {
                format!("assert refuses {}", self.with_prefix(15, subject))
            }
            Stmt::Expr(expr) => self.expr(expr),
        }
    }

    /// One element per line, for a list that did not fit.
    ///
    /// The elements are rendered here rather than handed in already rendered,
    /// because they are going one level further in than the line that tried to
    /// hold them, and an element that breaks across lines has to know that.
    /// Rendering at the outer indent and then pushing spaces in front only
    /// moves the first line, so `map(xs, |n: Int| { ... })` came out with the
    /// closure's body and its closing brace a level short of where they belong.
    ///
    /// Nothing is on the line either, so whatever prefix the caller was
    /// carrying is put down for the duration.
    fn broken<T>(
        &mut self,
        open: &str,
        items: &[T],
        close: &str,
        mut render: impl FnMut(&mut Self, &T) -> String,
    ) -> String {
        let mut text = format!("{open}\n");
        let pending = std::mem::take(&mut self.pending);
        self.indent += 1;
        for item in items {
            let element = render(self, item);
            text.push_str(&INDENT.repeat(self.indent));
            text.push_str(&element);
            text.push_str(",\n");
        }
        self.indent -= 1;
        self.pending = pending;
        text.push_str(&INDENT.repeat(self.indent));
        text.push_str(close);
        text
    }

    /// Whether a rendered fragment fits on the current line.
    ///
    /// Only the first line is measured, because a fragment that already breaks
    /// has made its own decision and re-measuring it would undo it.
    fn fits(&self, text: &str) -> bool {
        if text.contains('\n') {
            return false;
        }
        self.indent * INDENT.len() + self.pending + text.len() <= WIDTH
    }

    /// Renders `expr` knowing that `prefix` is already on the line.
    fn with_prefix(&mut self, prefix: usize, expr: &Expr) -> String {
        let saved = self.pending;
        self.pending = saved + prefix;
        let text = self.expr(expr);
        self.pending = saved;
        text
    }

    // -- types and patterns ------------------------------------------------

    fn ty(&self, ty: &Type) -> String {
        match ty {
            Type::Named { name, args, .. } => {
                if args.is_empty() {
                    name.name.clone()
                } else {
                    let rendered: Vec<String> = args.iter().map(|arg| self.ty(arg)).collect();
                    format!("{}<{}>", name.name, rendered.join(", "))
                }
            }
            Type::Fn {
                params, row, ret, ..
            } => {
                let rendered: Vec<String> = params.iter().map(|param| self.ty(param)).collect();
                // Before the arrow, which is where the parser insists on it:
                // after the return type it would be indistinguishable from a
                // declaration's own contract.
                let row = if row.is_empty() {
                    String::new()
                } else {
                    let named: Vec<String> = row.iter().map(effect_ref).collect();
                    format!(" uses {}", named.join(", "))
                };
                format!("Fn({}){row} -> {}", rendered.join(", "), self.ty(ret))
            }
            Type::Unit(_) => "()".to_string(),
            Type::Error(_) => String::new(),
        }
    }

    fn pattern(&mut self, pattern: &Pattern) -> String {
        match pattern {
            Pattern::Wildcard(_) => "_".to_string(),
            Pattern::Path { segments, .. } => path(segments),
            Pattern::Tuple {
                path: p, elements, ..
            } => {
                let rendered: Vec<String> = elements.iter().map(|e| self.pattern(e)).collect();
                format!("{}({})", path(p), rendered.join(", "))
            }
            Pattern::Record {
                path: p, fields, ..
            } => {
                let rendered: Vec<String> = fields.iter().map(|f| self.pattern_field(f)).collect();
                if rendered.is_empty() {
                    format!("{} {{}}", path(p))
                } else {
                    format!("{} {{ {} }}", path(p), rendered.join(", "))
                }
            }
            Pattern::Int { value, .. } => value.to_string(),
            Pattern::Str { value, .. } => string_literal(value),
            Pattern::Bool { value, .. } => value.to_string(),
            // On one line however many there are. An alternative binds
            // nothing, so each one is a name or a literal and the list stays
            // short enough to read; breaking it would put the `|` at the start
            // of a line, where it is the one place this language would rather
            // it did not appear.
            Pattern::OneOf { alternatives, .. } => {
                let rendered: Vec<String> = alternatives.iter().map(|p| self.pattern(p)).collect();
                rendered.join(" | ")
            }
            Pattern::Error(_) => String::new(),
        }
    }

    fn pattern_field(&mut self, field: &PatternField) -> String {
        match &field.pattern {
            Some(pattern) => format!("{}: {}", field.name.name, self.pattern(pattern)),
            None => field.name.name.clone(),
        }
    }
}

/// Appends a blank line, and never two.
fn blank_line(buffer: &mut String) {
    if !buffer.is_empty() && !buffer.ends_with("\n\n") {
        buffer.push('\n');
    }
}

fn path(segments: &[Ident]) -> String {
    segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// `<T, U>`, or `<A, B, uses r>`, or nothing at all.
///
/// Type parameters first and row variables after, whatever order they were
/// written in. One canonical form, and this is the one that reads: what a call
/// works out from the arguments, then what it works out from their rows.
///
/// Never broken across lines. A list long enough to need it would be a
/// declaration with a different problem.
fn type_params(generics: &[Ident], rows: &[Ident]) -> String {
    if generics.is_empty() && rows.is_empty() {
        return String::new();
    }
    let mut written: Vec<String> = generics
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect();
    written.extend(rows.iter().map(|row| format!("uses {}", row.name)));
    format!("<{}>", written.join(", "))
}

fn effect_ref(effect: &EffectRef) -> String {
    if effect.all {
        return format!("{}.*", effect.effect.name);
    }
    match &effect.operation {
        Some(operation) => format!("{}.{}", effect.effect.name, operation.name),
        None => effect.effect.name.clone(),
    }
}

/// Binding power, only used to decide where parentheses have to go back.
fn precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            3
        }
        BinaryOp::Add | BinaryOp::Sub => 4,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 5,
    }
}

/// Re-escapes a string, since the AST holds the decoded value.
fn string_literal(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('"');
    for c in value.chars() {
        match c {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\t' => text.push_str("\\t"),
            '\r' => text.push_str("\\r"),
            '\0' => text.push_str("\\0"),
            other => text.push(other),
        }
    }
    text.push('"');
    text
}
