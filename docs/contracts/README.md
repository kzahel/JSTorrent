# Contracts

These documents are the normative, human-readable protocol contracts for
interfaces implemented by more than one JSTorrent component:

- [`io-daemon-contract.md`](io-daemon-contract.md) defines the shared HTTP,
  WebSocket, capability, and lifecycle behavior of the IO daemons.
- [`native-host-contract.md`](native-host-contract.md) defines desktop native
  host bootstrap, profile, root, and takeover behavior.

The machine-readable definitions in the repository's
[`contracts/`](../../contracts/) directory and their conformance runners are
part of each contract. Change the prose, machine-readable definitions,
implementations, and tests together when contract behavior changes.
