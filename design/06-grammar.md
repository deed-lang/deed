# Grammar

The parser is the specification. This document states the same rules in notation a reader can
scan; the test in `crates/deed-parser/tests/design_grammar.rs` checks it against the parser
in both directions, so the two cannot drift apart silently. The same move is made everywhere
else in the repository: a claim in prose is checked against something.

The notation is mostly standard EBNF. `A*` means zero or more, `A+` means one or more, `A?`
means zero or one, `(A | B)` means a choice, and `"text"` is a literal terminal. The grammar
is presented bottom-up where that is clearer.

## Lexical grammar

### Source encoding

Files are UTF-8. A UTF-8 byte order mark at the start of a file is silently discarded; it
does not become a token.

### Trivia

Whitespace and comments are trivia. They are not part of the token stream. The formatter
preserves comments; nothing else in the pipeline sees them. Whether any trivia between two
tokens contained a newline is recorded in the `starts_line` field of the second token, which
is how the layout rules work (see below).

```
whitespace  ::=  any Unicode character with the `White_Space` property
line-comment ::=  "//" (any character other than newline)*
block-comment ::=  "/*" block-comment-content* "*/"
block-comment-content ::=  block-comment | (any character other than "/*" and "*/")
```

Block comments nest: `/* outer /* inner */ still outer */` is one comment.

### Tokens

```
token  ::=
    keyword | ident | integer | string |
    "(" | ")" | "{" | "}" | "[" | "]" |
    "," | "." | ":" | ";" | "?" | "_" | "|" |
    "->" | "=>" |
    "=" | "==" | "!=" | "!" |
    "<" | "<=" | ">" | ">=" |
    "+" | "-" | "*" | "/" | "%" |
    "&&" | "||"
```

### Identifiers and keywords

An identifier starts with a Unicode letter or `_` and continues with Unicode letters, digits,
and `_`. The single character `_` is the wildcard token rather than an identifier; it cannot
be used as a name.

A string of identifier characters that matches a keyword lexes as that keyword rather than an
identifier.

```
ident         ::=  ident-start ident-continue*
ident-start   ::=  "_" | unicode-letter
ident-continue ::=  "_" | unicode-letter | unicode-digit

keyword  ::=
    "assert"  | "choice"     | "deprecated" | "effect"  | "else"      | "ensures"
  | "false"   | "fn"         | "for"     | "handler"   | "if"
  | "implements" | "in"      | "let"     | "match"     | "module"
  | "old"     | "record"     | "return"  | "test"      | "true"
  | "type"    | "unchanged"  | "use"     | "uses"      | "where"
  | "with"
```

Here `unicode-letter` and `unicode-digit` mean the Unicode Letter and Decimal_Digit
properties.

The following words are contextual: they lex as ordinary identifiers and are read by the
parser in specific positions only. A variable may still be named any of them.

```
soft-keyword  ::=  "at"  |  "err"  |  "finally"  |  "ok"  |  "refuses"  |  "state"  |  "while"
```

### Integer literals

```
integer      ::=  decimal | hex | binary | octal
decimal      ::=  digit (digit | "_")*
hex          ::=  "0" ("x" | "X") hex-digit (hex-digit | "_")*
binary       ::=  "0" ("b" | "B") ("0" | "1" | "_")+
octal        ::=  "0" ("o" | "O") octal-digit (octal-digit | "_")*
digit        ::=  "0" ... "9"
hex-digit    ::=  digit | "a" ... "f" | "A" ... "F"
octal-digit  ::=  "0" ... "7"
```

Digit separators (`_`) are accepted anywhere after the first digit and are stripped before
the value is decoded. `Int` is a signed 64-bit integer, so values range from
`-9223372036854775808` to `9223372036854775807`. A digit run is decoded before any unary `-`
is applied, which means a positive literal must fit `0` to `9223372036854775807`; a larger
digit run is a lexical error.

Numeric literals use ASCII digits only. Unicode digits are accepted in identifiers but not in
integer literals.

There are no float literals.

### String literals

```
string     ::=  '"' string-char* '"'
string-char ::=  escape | (any character other than '"', '\', and newline)
escape     ::=  '\\' escape-code
escape-code ::=  'n' | 't' | 'r' | '0' | '\\' | '"' | 'u' '{' hex-digit+ '}'
```

String literals do not span lines. The escape sequences are:

| Sequence     | Meaning                                      |
| ------------ | -------------------------------------------- |
| `\n`         | newline (U+000A)                             |
| `\t`         | horizontal tab (U+0009)                      |
| `\r`         | carriage return (U+000D)                     |
| `\0`         | null (U+0000)                                |
| `\\`         | backslash                                    |
| `\"`         | double quote                                 |
| `\u{hex}`    | Unicode scalar value (0 to 10FFFF, not D800-DFFF) |

## Context-free grammar

### Module structure

Every file begins with a `module` declaration followed by zero or more `use` declarations and
then the body declarations.

```
module       ::=  "module" module-path module-edition? use* item*
module-edition ::= "edition" integer

module-path  ::=  ident ("/" ident)*

use          ::=  "use" module-path "." "{" ident ("," ident)* ","? "}"
```

`use` names are explicit and there are no wildcards. A `use` may only appear before the first
`item`.

### Items

```
item  ::=
    deprecate-decl
  | type-alias
  | record-decl
  | choice-decl
  | effect-decl
  | handler-decl
  | fn-decl
  | test-decl
```

```
deprecate-decl  ::=  "deprecated" ident "->" ident
```

#### Type alias

```
type-alias  ::=  "type" ident type-params? "=" type ("where" expr)?
```

The optional `where` clause is a refinement: a boolean expression over the type's values that
must hold for every inhabitant of the alias.

#### Record

```
record-decl   ::=  "record" ident type-params? "{" field-list "}"
field-list    ::=  (field-decl ("," field-decl)* ","?)?
field-decl    ::=  ident ":" type
```

#### Choice

```
choice-decl  ::=  "choice" ident type-params? "{" variant-list "}"
variant-list ::=  (variant ("," variant)* ","?)?
variant      ::=  ident ("{" field-list "}")?
```

Variant payloads are named, not positional. `Circle(Int)` is not a valid variant.

#### Effect

```
effect-decl  ::=  "effect" ident "{" fn-sig* "}"
```

An effect declares operation signatures only; it has no bodies.

#### Handler

```
handler-decl    ::=  "handler" ident "implements" ident "{" handler-member* "}"
handler-member  ::=  state-field | fn-decl | finally-block
state-field     ::=  "state" ident ":" type
finally-block   ::=  "finally" block
```

`state` is a soft keyword: it introduces a mutable field in exactly this position and is an
ordinary name everywhere else.

`finally` is a soft keyword: it introduces the cleanup block in exactly this position and is an
ordinary name everywhere else. A handler may have at most one `finally` block. It runs whenever
the `with` block that installed the handler exits, whether the body returned normally, returned
early, or a contract failed. The block can read and write handler state.

#### Function

```
fn-decl   ::=  fn-sig contract block
fn-sig    ::=  "fn" ident decl-params? "(" param-list ")" ("->" type)?
param-list ::=  (param ("," param)* ","?)?
param      ::=  ident (":" type)?
```

A parameter type may be omitted only in a handler operation body, where the effect already
declared the signature. Every other parameter must have a type.

#### Type parameters

```
decl-params  ::=  "<" decl-param ("," decl-param)* ","? ">"
decl-param   ::=  ("uses" ident) | ident
```

Type parameters are ordinary names; row variables (effect parameters) are written `uses Name`.
Type arguments only appear right after a declaration name, where `<` cannot be a comparison.

#### Test

```
test-decl  ::=  "test" string block
```

### Types

```
type  ::=
    "()"
  | "Fn" "(" type-list ")" ("uses" effect-ref ("," effect-ref)* ","?)? "->" type
  | ident ("<" type-list ">")?

type-list  ::=  (type ("," type)* ","?)?
```

`Fn` is the function type. The `uses` clause in a function type lists the effects the function
may perform; it comes before `->` so that `fn f() -> Fn(Int) -> Int uses Log` is not
ambiguous between a return type and a clause on the declaration itself.

### Contracts

A contract appears between a function signature and its body. The three clauses must appear
in the order `where`, then `uses`, then `ensures`, and each may appear at most once.

```
contract  ::=  where-clause? uses-clause? ensures-clause?

where-clause    ::=  "where" expr ("," expr)* ","?
uses-clause     ::=  "uses" effect-ref ("," effect-ref)* ","?
ensures-clause  ::=  "ensures" ensures ("," ensures)* ","?

effect-ref  ::=  ident ("." (ident | "*"))?

ensures  ::=  ("ok" | "err") "=>" expr
```

`ok` and `err` are soft keywords: they name an outcome in this position and are ordinary
names everywhere else.

### Blocks and statements

```
block  ::=  "{" stmt* expr? "}"
```

A block's value is the trailing expression, if there is one. Statements are separated by
nothing, with one exception described in the layout rules below. An optional `;` is accepted
after any statement and ignored.

```
stmt  ::=
    "let" pattern (":" type)? "=" expr
  | "return" expr?
  | "assert" expr
  | "assert" "refuses" expr
  | ident "=" expr
  | expr
```

`refuses` is a soft keyword: it marks an assertion that expects a contract violation in
exactly this position. The lookahead that distinguishes `assert refuses f(x)` (a refuses
assertion) from `assert refuses(x)` (an assertion about a call to `refuses`) is that
`refuses` is followed by an identifier, not `(`.

### Expressions

The expression grammar uses standard operator precedence. Postfix operators have the highest
precedence; prefix unary operators come next; then binary operators in the order below.

```
expr  ::=
    expr binary-op expr            -- binary, left-associative
  | unary-op expr                  -- prefix unary
  | expr "." ident                 -- field access
  | expr "(" arg-list ")"          -- call
  | expr "?"                       -- try (propagate error)
  | expr "{" field-init-list "}"   -- struct literal (context-dependent, see below)
  | "()"                           -- unit
  | "(" expr ")"                   -- grouping
  | "[" arg-list "]"               -- list literal
  | "{" stmt* expr? "}"            -- block
  | "|" param-list "|" expr        -- closure
  | "||" expr                      -- closure with no parameters
  | "if" expr block ("else" ("if" expr block | block))?
  | "match" expr "{" match-arm-list "}"
  | "for" ident ("at" ident)? "in" expr
        ("with" ident "=" expr)?
        ("while" expr)?
        block
  | "with" expr ("," expr)* ","? block
  | "old" "(" expr ")"
  | "unchanged" "(" effect-ref ")"
  | integer
  | string
  | "true" | "false"
  | ident

arg-list         ::=  (expr ("," expr)* ","?)?
field-init-list  ::=  (field-init ("," field-init)* ","?)?
field-init       ::=  ident (":" expr)?
match-arm-list   ::=  (match-arm ("," match-arm)* ","?)?
match-arm        ::=  arm-pattern "=>" expr
```

#### Binary operator precedence

Lower number binds less tightly.

| Precedence | Operators               |
| ---------- | ----------------------- |
| 1          | `\|\|`                  |
| 2          | `&&`                    |
| 3          | `==`  `!=`  `<`  `<=`  `>`  `>=` |
| 4          | `+`  `-`                |
| 5          | `*`  `/`  `%`           |

```
binary-op  ::=  "||" | "&&" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | "%"
unary-op   ::=  "-" | "!"
```

#### `at` and `while` in `for`

`at` introduces an index binder and `while` introduces an early-stop condition. Both are soft
keywords: they lex as identifiers and are read by name in the one position where nothing else
could stand.

#### Struct literals

A `{` that follows a path expression is a struct literal in most positions. It is not when the
parser is reading an `if` or `match` scrutinee, because `if x { }` would otherwise be a
literal rather than a conditional. In a `with` handler list and in a `for` iterable or
accumulator, `{` opens a struct literal only when it is immediately followed by `name:` or is
an empty `{}` followed by a second `{`.

#### `old` and `unchanged`

`old` evaluates its argument in the pre-call state; `unchanged` asserts that the named effect
performs no state transitions. Both appear only in `ensures` obligations, though the parser
accepts them anywhere an expression may appear.

### Patterns

```
pattern  ::=
    "_"
  | integer
  | string
  | "true" | "false"
  | path ("(" pattern-list ")")?    -- tuple variant pattern
  | path ("{" pattern-field-list "}")?  -- record pattern
  | path                            -- variable or path

arm-pattern  ::=  pattern ("|" pattern)*

path               ::=  ident ("." ident)*
pattern-list       ::=  (pattern ("," pattern)* ","?)?
pattern-field-list ::=  (pattern-field ("," pattern-field)* ","?)?
pattern-field      ::=  ident (":" pattern)?
```

Alternatives (`|`) are only allowed in match arm patterns, not in `let` or parameter
patterns, because those forms bind names and a pattern with alternatives binds nothing.

## Layout rules

Deed has no statement terminator. A statement ends when the next token cannot continue the
expression it is parsing, with one exception: tokens that can both start an expression and
continue one are treated as starting a new statement when they appear at the beginning of a
line.

The rule is tracked by the `starts_line` field on each token. A token has `starts_line = true`
when any trivia (whitespace or comments) between it and the previous token contained a newline.
The formatter never breaks a binary expression or places a call's opening parenthesis on a line
of its own, so canonical formatting never triggers these rules.

The specific cases where `starts_line` changes the parse:

- **Binary operators.** A binary operator that starts a line ends the expression and begins a
  new statement rather than continuing the one before it. Writing `a + b` on one line and then
  `-c` on the next means `-c` is a new expression (unary negation), not `a + b - c`. Leave
  the operator on the first line to carry the expression over.

- **Call parenthesis.** A `(` that starts a line is not a call on the expression above it. It
  begins a new grouped expression.

- **Field access, `?`, and struct literals.** A `.`, `?`, or `{` that starts a line does not
  continue the expression before it.

The rule is uniform: it is not switched off inside brackets. "An expression ends at the end
of a line" is one sentence, and the version with exceptions would be three.
