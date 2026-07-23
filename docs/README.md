# Documentation

JSTorrent keeps only documentation with a clear current role at the active
level:

- [`topics/`](topics/README.md) contains living records for continuing
  concerns. These documents own current status, decisions, gaps, and next work.
- [`contracts/`](contracts/README.md) contains normative protocol contracts
  backed by machine-readable definitions and conformance tests.
- [`reference/`](reference/README.md) contains useful external or imported
  reference material whose age and provenance are stated explicitly.
- [`archive/`](archive/README.md) preserves historical plans, designs,
  investigations, reports, and project snapshots for context.

When guidance changes, update the relevant topic or contract instead of adding
another point-in-time plan. Archive material can explain why something was
built, but it must not be treated as the current source of truth without
reconciling it against the implementation.

Run `pnpm docs:check` after changing active documentation. The check validates
local links, referenced repository scripts, and accidental machine-specific
absolute paths.
