# Topics

Focused, living records of continuing concerns live here.

A topic owns the current truth for a concern whose decisions, evidence, gaps,
and next work need to survive across sessions or commits. Topics may cover
contracts, recurring problems, product decisions, implementation campaigns,
status questions, or investigations. Prefer the smallest coherent topic whose
direction can evolve independently.

Create or update a topic when:

- Work spans multiple commits or tactical slices.
- Important decisions or invariants need a durable home.
- New evidence changes the current direction.
- The current status is otherwise difficult to answer.
- The user explicitly asks for a living topic.

Do not create a topic for every small standalone change.

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
- [`ios-altstore-pal-distribution.md`](ios-altstore-pal-distribution.md):
  current iOS alternative-distribution release workflow, safety invariants,
  recovery path, and CI code map.
- [`sandbox-and-search-plugin-trust-boundaries.md`](sandbox-and-search-plugin-trust-boundaries.md):
  platform sandbox boundaries and the security and Google Play implications of
  installable search plugins.
- [`search-plugins.md`](search-plugins.md): current plugin manifest, runtime,
  result, and reference-implementation contract.
