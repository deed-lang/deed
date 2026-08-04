const KEYWORDS = [
  "module",
  "use",
  "deprecated",
  "type",
  "record",
  "choice",
  "effect",
  "handler",
  "finally",
  "implements",
  "fn",
  "where",
  "uses",
  "ensures",
  "old",
  "unchanged",
  "let",
  "if",
  "else",
  "match",
  "for",
  "in",
  "return",
  "true",
  "false",
  "test",
  "with",
  "assert",
  "abandon",
  "finally",
  "state",
  "at",
  "while",
  "refuses",
  "ok",
  "err",
  "operator",
  "from",
];

module.exports = grammar({
  name: "deed",

  word: ($) => $.identifier,

  extras: () => [/\s/],

  rules: {
    source_file: ($) =>
      repeat(
        choice(
          $.comment,
          $.string,
          $.number,
          $.keyword,
          $.type_name,
          $.identifier,
          $.operator,
          $.punctuation,
        ),
      ),

    comment: () => token(seq("//", /.*/)),

    string: ($) =>
      seq(
        '"',
        repeat(choice(token.immediate(/[^"\\\n]+/), $.escape)),
        '"',
      ),

    escape: () => token.immediate(seq("\\", /./)),

    number: () => /\b[0-9]+\b/,

    keyword: () => choice(...KEYWORDS),

    type_name: () => /[A-Z][A-Za-z0-9_]*/,

    identifier: () => /[a-z_][A-Za-z0-9_]*/,

    operator: () =>
      token(
        choice(
          "=>",
          "==",
          "!=",
          "<=",
          ">=",
          "->",
          "&&",
          "||",
          "?",
          "+",
          "-",
          "*",
          "/",
          "%",
          "<",
          ">",
          "=",
          "!",
        ),
      ),

    punctuation: () => /[()\[\]{}:.,]/,
  },
});
