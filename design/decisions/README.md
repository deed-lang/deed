# Decision records

This directory is where design proposals and large design decisions live.

Create a new file under `design/decisions/` for each proposal or accepted decision.
Use this name format:

`YYYY-MM-DD-short-title.md`

Start from [`TEMPLATE.md`](TEMPLATE.md).

Required sections in every decision record:

- Drawbacks
- Rejected Ideas
- Open Questions

Rejected Ideas must explain why each option was not chosen so the same proposal does not need to be reopened without new evidence.

If a decision changes later, update the original record in the same PR:

- set its status to `Superseded`
- add `Superseded by: <link to new decision record>`
- keep the old reasoning in place

The newer decision record should include `Supersedes: <link to old decision record>`.
