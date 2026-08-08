<!--
One PR, one concern. If this closes an issue, say `Closes #N` somewhere below and GitHub
will link them.
-->

## What this changes

<!-- The problem first, then the decision you made about it. The diff shows the rest. -->

## Why this way

<!--
The part a reader cannot get from the diff. If you considered something else and rejected
it, that is worth a line, especially if the rejected version looks more obvious.
-->

## What it still does not do

<!--
Optional, and the most useful section in most PRs here. A limitation written down is a
limitation somebody can argue with. A limitation left out is one somebody discovers later
and assumes was an oversight.
-->

## Checks

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo run -p deed-lang -- check examples/` and `test examples/`, if this touches the compiler
- [ ] A design document under `design/` updated in this PR, if this changes behaviour one of
      them describes. A design that lags the code is worse than no design.
- [ ] Tests added for the thing that was wrong, not only for the thing that is now right. A
      guard test that only passes values the guard accepts proves nothing, and that is how
      the worst bug here went unnoticed.

<!--
If you used AI assistance, say so in a line at the bottom. Nobody minds. What is not fine is
opening a PR you have not read and understood yourself, since review being expensive is the
entire premise of this project.
-->
