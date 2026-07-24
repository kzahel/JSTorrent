# Repository Instructions

`AGENTS.md` points here so repository automation shares one instruction source.

## Cross-Project Context

For maintainer-specific cross-project context, see
`~/code/dotfiles/projects/README.md` when that checkout is available.

## ChromeOS Hardware Testing

The authoritative physical-device controller is the separate checkout at
`~/code/chromeos-testbed`; do not add or revive a copied testbed inside this
repository. Before ChromeOS hardware work, read
`~/code/chromeos-testbed/skills/SKILL.md` and
[`docs/topics/chromeos-hardware-testing.md`](docs/topics/chromeos-hardware-testing.md).

Start with `~/code/chromeos-testbed/bin/chromeos doctor`. Use `chromeroot` for
the ChromeOS host, UI automation, screenshots, DevTools, extension deployment,
and ARCVM ADB. `chromebook` is the optional Crostini container; start it only
when Linux-container access or shared files are actually needed.

Use `./scripts/deploy-chromebook.sh` for the extension and
`./scripts/deploy-android-chromebook.sh` for the APK. Do not follow ChromeOS
deployment procedures under `docs/archive/` without reconciling them against
the living topic and current scripts.

## Documentation Ownership

Active documentation has four roles:

- `README.md` and `DEVELOPMENT.md` are the product and maintainer entry points.
- Platform and package `README.md` files own current build, test, architecture
  summary, and code-map information for their directory.
- `docs/topics/` owns current truth for continuing cross-cutting concerns.
- `docs/contracts/` owns normative protocol behavior.

Historical plans, designs, and investigations belong under `docs/archive/` and
must not be treated as current without reconciliation against the code.

Before changing a continuing concern, read its topic. Update the topic when the
work changes its status, decisions, evidence, validation, gaps, or recommended
direction. Do not create a topic for every standalone change. See
`docs/topics/README.md`.

## Commit Messages

Aim for a subject of 65 characters or fewer and strictly wrap commit bodies at
72 columns. Keep the subject as a scannable result; put the motivation in the
body.

For nontrivial commits, preserve the originating intent rather than only
listing the diff. Capture the desired outcome, important constraints and
non-goals, implementation direction, validation, and deliberate deferrals when
they help a future maintainer reconstruct the change.

Prune secrets, transcript detail, and low-signal commentary. Do not mention
Claude, AI, or an AI assistant. Do not add AI co-author or generation trailers.

Mechanical changes may use a one-line message. When a commit materially
advances a living topic, append one or more exact `Topic: <slug>` trailers.

## Toolchain Setup

On configured development machines, source the shell profile before commands
that require Java, Android, Rust, or other locally installed tools:

```bash
source ~/.profile
```

Install workspace dependencies normally:

```bash
pnpm install
```

When a Python directory has `pyproject.toml` or `uv.lock`, run its scripts and
tests through `uv` instead of using globally installed packages.

For a local sibling `playsvideo` checkout, keep the published dependency
committed and relink only in the local workspace:

```bash
pnpm --dir packages/client link ../playsvideo
```

## Validation

Choose validation in proportion to the change and report exactly what ran.

TypeScript:

```bash
pnpm typecheck
pnpm test
pnpm lint
pnpm format
```

Use `pnpm format:fix` only when formatting needs to be rewritten.

Rust changes under `desktop/`:

```bash
cd desktop
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Android Kotlin/Java changes:

```bash
cd android
./gradlew :app:compileDebugKotlin
./gradlew testDebugUnitTest
```

iOS Swift or project changes:

```bash
swift test --package-path ios/JSTorrentKit
```

Documentation changes:

```bash
pnpm docs:check
pnpm format
```

Remove temporary logs and other investigation artifacts before finishing.

Package and platform READMEs contain focused commands for instrumented,
end-to-end, conformance, simulator, and deployment testing.

## Durable Engineering Invariants

- External info-hash strings use `infoHashFromHex`; binary hashes use
  `infoHashFromBytes`. Never cast arbitrary strings to `InfoHashHex`.
- QuickJS native boolean results may arrive as `"true"` or `"false"` strings.
  Callers of `__jstorrent_*` functions must compare explicitly instead of
  relying on truthiness.
- Shared native-host and IO-daemon behavior changes require corresponding
  contract definitions and conformance tests.
- The extension engine runs in the foreground UI page, not the MV3 service
  worker.
- Android and iOS run the engine in-process with native bindings; their native
  UIs do not use `@jstorrent/client`.

## Releases

Release scripts commit, push, and tag. Run one only when the user explicitly
requests a release, and read `docs/topics/releases.md` first. The iOS-specific
notarization and recovery contract is in
`docs/topics/ios-altstore-pal-distribution.md`.

## Git Attribution

Before pushing, verify `git config user.name` and `git config user.email`.
Stop if they are automation placeholders such as `Claude` or
`noreply@anthropic.com`; commits must use the maintainer's configured identity.
