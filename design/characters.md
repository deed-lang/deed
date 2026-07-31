# Characters

Deed keeps characters as one-character `String`s for now. This change does not add a
`Character` type, and it does not add a code point primitive.

## Why not a `Character` type

`split(text, "")` already yields the unit every string operation here uses. `std/string`
walks one-character strings today for slicing, trimming, and its ASCII `to_upper` and
`to_lower` tables. A separate `Character` type would duplicate that representation and add
conversions at every boundary without making the current library simpler or more precise.

That answer is good enough for code that only needs equality, concatenation, or a table over
characters written in the source. It is not good enough for a library that wants to classify
or map arbitrary text.

## Why not a code point primitive

A code point primitive would be the first operation that distinguishes an arbitrary
character from every other one without somebody spelling an alphabet out by hand. That would
make a fully table-driven case library writable, and it would let a user-written comparator
rank any character it can receive.

Deed is not taking that step in this change because the pressure is narrower than the old
arguments claimed. Text ordering is not impossible without `<` on `String`, and case
conversion is not impossible without a new primitive. The language already has both in
limited form.

## Consequences

`String` keeps `<`, `<=`, `>`, and `>=`.

The reason is not that text would otherwise be unorderable. `split(s, "")` already makes a
text comparator writable. The real problem is that the writable version is partial unless
each program writes its own alphabet, and every character left out of that alphabet silently
ties with every other omitted one. Keeping `<` on `String` avoids that repeated, easy to
miss bug, which is why text stays different from the refused cases for records and bare type
parameters.

Case conversion is already writable for the finite table Deed ships today. `std/string`
implements `to_upper` and `to_lower` as the twenty-six ASCII letters and documents that
every other character passes through unchanged.

General case conversion is still not "just a library table". Without a code point or an
equivalent primitive, a library can only key a table by one-character strings that were
typed into the program. That is enough for ASCII and any other alphabet written out by hand.
It is not enough for "whatever character the input contains".

## What would change this

If Deed needs a library that classifies or maps arbitrary characters rather than a fixed
hand-written alphabet, add the smallest primitive that exposes a stable code point for a
one-character string. Revisit a separate `Character` type only if that primitive proves
awkward at string boundaries often enough to justify the extra type.

AI assistance: Drafted with GitHub Copilot and reviewed by the author.
