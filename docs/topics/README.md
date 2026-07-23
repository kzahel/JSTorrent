# Topics

Focused, living records of continuing concerns live here.

A topic owns the current truth for a concern whose decisions, evidence, gaps,
and next work need to survive across sessions or commits. Topics may cover
contracts, recurring problems, product decisions, implementation campaigns,
status questions, or investigations. Prefer the smallest coherent topic whose
direction can evolve independently.

Adopt this convention incrementally. Existing architecture, design, research,
and plan documents do not need to move here solely for consistency. Create or
update a topic when:

- Work spans multiple commits or tactical slices.
- Important decisions or invariants need a durable home.
- New evidence changes the current direction.
- The current status is otherwise difficult to answer.
- The user explicitly asks for a living topic.

Do not create a topic for every small standalone change.

## Documentation Roles

- Architecture, design, and contract documents own durable system shape.
- Research and investigation documents preserve evidence and findings.
- Plan documents describe bounded proposed or scheduled execution.
- Topic documents own current status, decisions, evidence, gaps, and direction
  for a continuing concern.

## Topic Shape

New topics should normally include:

- A clear title.
- A stable `Topic: <slug>` line matching the filename.
- An honest status and, when useful, a last-reconciled date.
- Only the sections the concern needs, such as scope, current state, decisions,
  invariants, evidence, code map, known gaps, and recommended next work.

Update the existing topic instead of creating date-stamped replacements when
the same concern evolves. When a commit materially advances a topic, use the
same slug in a `Topic: <slug>` commit-message trailer where practical.

## Current Topics

- [`android-api-37-local-network-permission.md`](android-api-37-local-network-permission.md):
  deferred migration record for Android 17 local-network enforcement, including
  affected JSTorrent paths, permission UX, implementation boundaries, and
  release acceptance criteria.
