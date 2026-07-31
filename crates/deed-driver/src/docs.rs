//! The API reference a module carries, read off the module.
//!
//! There are no visibility modifiers in this language: every item a module
//! declares is exported, so every declaration is API and there is nothing to
//! decide about what belongs on a page. That makes generating one cheap enough
//! that it should not be a thing only the shipped library gets.
//!
//! `docs/std.md` used to say the same treatment was "possible for user modules
//! in principle", which is the kind of sentence this repository exists to stop
//! writing. `deed doc` is the sentence stopping being in principle.
//!
//! Nothing here decides policy. A function with no comment above it produces a
//! page with no description rather than an error, because a person running
//! `deed doc` on their own module is not asking to be told what to write.
//! `crates/deed-driver/tests/documentation.rs` is where the shipped library is
//! held to a stricter rule, which is the right place for it: that is a claim
//! about this repository rather than about the language.

use deed_ast::{EffectRef, FnDecl, Item, TestDecl};
use deed_diagnostics::Span;

use crate::Checked;

/// One function's entry on a page.
pub struct FunctionDocs {
    pub name: String,
    /// The comment block above the declaration, or `None` when there is none.
    pub description: Option<String>,
    /// The signature as it is written, quoted rather than printed from the
    /// tree: what a reader has to match is the text in the file.
    pub signature: String,
    /// Row variables the signature introduces, in order.
    pub row_variables: Vec<String>,
    /// The row the declaration carries, empty when the function is pure.
    pub declared_row: Vec<String>,
    /// The contract between the signature and the body, empty when there is
    /// none.
    pub contract: String,
    /// One entry per test that names this function, with the lines that do.
    pub examples: Vec<(String, String)>,
}

/// One module's page.
pub struct ModuleDocs {
    pub module: String,
    /// The file's own header comment, or `None` when it has none.
    pub summary: Option<String>,
    pub functions: Vec<FunctionDocs>,
}

impl ModuleDocs {
    /// Everything a page can say about one checked module.
    pub fn of(checked: &Checked, text: &str) -> ModuleDocs {
        let module = checked
            .module
            .name
            .as_ref()
            .map(|name| name.to_string_path())
            .unwrap_or_default();

        let functions = checked
            .module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(FunctionDocs::of(checked, text, function)),
                _ => None,
            })
            .collect();

        ModuleDocs {
            module,
            summary: non_empty(file_header_comment(text)),
            functions,
        }
    }

    /// The page as Markdown.
    ///
    /// `from` is what the page says it was generated from, which differs
    /// between a shipped module and a file on disk and is the caller's to
    /// know.
    pub fn to_markdown(&self, from: &str) -> String {
        let summary = self
            .summary
            .clone()
            .unwrap_or_else(|| "This module carries no header comment.".to_string());

        let body: Vec<String> = self
            .functions
            .iter()
            .map(|function| function.to_markdown(&self.module))
            .collect();

        let body = match body.is_empty() {
            true => "This module declares no functions.".to_string(),
            false => body.join("\n\n"),
        };

        format!(
            "# `{}`\n\n\
             _Generated from `{from}` and the module's own tests._\n\n\
             ## Module\n\n\
             {summary}\n\n\
             {body}\n",
            self.module
        )
    }
}

impl FunctionDocs {
    fn of(checked: &Checked, text: &str, function: &FnDecl) -> FunctionDocs {
        let tests: Vec<&TestDecl> = checked
            .module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Test(test) => Some(test),
                _ => None,
            })
            .collect();

        // Mentions come from the resolver rather than from the text, so a name
        // inside a string or a comment is not an example of anything.
        let mut mentions: Vec<Span> = match checked.resolutions.resolution(function.sig.name.span) {
            Some(def) => checked
                .resolutions
                .names()
                .filter_map(|(span, mention)| (mention == def).then_some(span))
                .collect(),
            None => Vec::new(),
        };
        mentions.sort_by_key(|span| (span.start, span.end));

        let mut examples = Vec::new();
        for test in tests {
            let inside: Vec<Span> = mentions
                .iter()
                .copied()
                .filter(|span| span.start >= test.span.start && span.end <= test.span.end)
                .collect();
            if inside.is_empty() {
                continue;
            }
            examples.push((test.name.clone(), example_lines(text, &inside)));
        }

        let contract = text[function.sig.span.end as usize..function.body.span.start as usize]
            .trim()
            .to_string();

        FunctionDocs {
            name: function.sig.name.name.clone(),
            description: non_empty(comment_block_before(text, function.sig.span)),
            signature: text[function.sig.span.as_range()].trim().to_string(),
            row_variables: function
                .sig
                .rows
                .iter()
                .map(|row| row.name.clone())
                .collect(),
            declared_row: function.contract.uses.iter().map(effect_ref).collect(),
            contract,
            examples,
        }
    }

    fn to_markdown(&self, module: &str) -> String {
        let description = self
            .description
            .clone()
            .unwrap_or_else(|| "No description.".to_string());
        let row_variables = match self.row_variables.is_empty() {
            true => "none".to_string(),
            false => self.row_variables.join(", "),
        };
        let declared_row = match self.declared_row.is_empty() {
            true => "pure".to_string(),
            false => self.declared_row.join(", "),
        };
        let contract = match self.contract.is_empty() {
            true => "pure".to_string(),
            false => self.contract.clone(),
        };

        let examples: String = self
            .examples
            .iter()
            .map(|(name, lines)| format!("#### `{name}`\n\n```deed\n{lines}\n```\n\n"))
            .collect();
        let examples = match examples.is_empty() {
            true => "No test names this function.".to_string(),
            false => examples.trim_end().to_string(),
        };

        format!(
            "## `{}`\n\n\
             ### Behavior and limits\n\n\
             {description}\n\n\
             ### Signature\n\n\
             ```deed\n{}\n```\n\n\
             ### Row variables\n\n\
             `{row_variables}`\n\n\
             ### Declared row\n\n\
             `{declared_row}`\n\n\
             ### Contract\n\n\
             ```deed\n{contract}\n```\n\n\
             ### Examples from `{module}.deed`\n\n\
             {examples}",
            self.name, self.signature
        )
    }
}

fn non_empty(text: String) -> Option<String> {
    match text.is_empty() {
        true => None,
        false => Some(text),
    }
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |at| at + 1)
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at)
}

/// The `//` block a file opens with, before the first blank-line-separated
/// gap that is followed by something other than a comment.
pub fn file_header_comment(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if lines.is_empty() {
                continue;
            }
            lines.push(String::new());
            continue;
        }
        let Some(comment) = trimmed.strip_prefix("//") else {
            break;
        };
        lines.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

/// The `//` block immediately above a declaration, with no blank line between.
pub fn comment_block_before(text: &str, span: Span) -> String {
    let mut found = Vec::new();
    let mut in_block = false;

    for line in text[..span.start as usize].lines().rev() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if in_block {
                break;
            }
            continue;
        }
        let Some(comment) = trimmed.trim_start().strip_prefix("//") else {
            break;
        };
        in_block = true;
        found.push(comment.strip_prefix(' ').unwrap_or(comment).to_string());
    }

    found.reverse();
    found.join("\n").trim().to_string()
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

fn example_lines(text: &str, mentions: &[Span]) -> String {
    let mut lines = Vec::new();
    for span in mentions {
        let start = line_start(text, span.start as usize);
        let end = line_end(text, span.end as usize);
        let line = text[start..end].trim().to_string();
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines.join("\n")
}
