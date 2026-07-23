# Engine Scripts

The package scripts fall into three groups:

- `validate-*.ts`: compare source implementations with machine-readable
  native-binding, native-host, and IO-daemon contracts.
- `run-*-conformance.ts`: execute the cross-implementation conformance suites.
- `download_*.ts` and `repro_logging.ts`: local diagnostics and benchmarks.

Prefer the package commands declared in
[`package.json`](../package.json), such as `conformance:daemon`,
`conformance:native-host`, `download-memory`, and `download-null`, rather than
invoking validation scripts with hand-written runtime flags.
