# Amoeba Program

This generated tree contains the reviewed on-chain source for the Amoeba Program
program. It is produced from private `ameba_spread/main` by an exact allowlist;
it is not a second implementation or a development workspace.

The current channel is a **pre-production DevNet source preview**. It is not a
production or mainnet release, does not contain deployment tooling, and does
not include an official `light_token_minter.so` binary.

## Included

- the production Rust modules for `light_token_minter`
- a production-only Cargo manifest and locked dependency graph
- the reviewed DevNet deployment manifest and v0.1.0 DevNet release intent
- the non-secret, hash-pinned V2 DevNet bootstrap policy
- the exact governed RAMX and NANDX product-SKU manifests, maintained-source
  provenance, and separate NANDX benchmark basket
- public source CI and security-reporting guidance
- `PUBLIC_SOURCE_PROVENANCE.json`, binding the export to its private source
  commit, policy hash, file count, normalized modes, and exact tree digest

## Deliberately excluded

- Rust and TypeScript tests, harnesses, fixtures, and simulated faucets
- TypeScript operator builders, automation, and release tooling
- private runbooks, point-in-time audits, operator procedures, and deploy helpers
- keypairs, credentials, build receipts, `.so` files, and generated bundles

The compiled program id is
`9ipkBCjEfeJDMF6AFrezRmDDHmbnmeyv45cfXNqAnWsH`. The generated
`deployments/devnet.json` manifest pins that id, Solana DevNet, its genesis
hash, the external collateral mint, and the create-once current-v2 bootstrap
policy for an absent namespace. Its binary, ProgramData, extension, upgrade, and
current-state compatibility evidence remain empty until an exact release
candidate passes every private gate; while they are empty, promotion is
explicitly blocked.

The public RAMX manifest is an audit source, not evidence of deployment. Version
1 preserves exactly 52 labels, encodes exact-case printable ASCII/UTF-8 labels as
right-zero-padded bytes32, sorts the encoded values, and commits root
`52a574e7fee12f9921b3b220b709be16c13a6f290abbdc2cc203ed1191ecf578`.
Its tag-190 submission plan remains `16/16/16/4`.

The public NANDX manifest independently governs exactly 48 terminal MPNs with
root `41bb5dc79bacabb3e0876b55e647b01084c7fe8b39846fd707ee0fd973d28c3d`,
source/PDF SHA-256
`14e31bd83e0c92c5b3894df5d68aec1091373c0ae48b53775d12640b92cb1be7`, and
tag-190 chunks `16/16/16`. The separate benchmark basket preserves 22 fixed rows
(18 item-addressable and 4 assessment-only) totaling exactly `100.00%`; terminal
identities carry no individual weight and monthly oracle source-recipe weights
remain separate. No transaction, signature, pause, or deployment is performed
by publishing this source preview.

## Inspect the source

```bash
cargo check \
  --manifest-path programs/light_token_minter/Cargo.toml \
  --locked \
  --lib
```

Any locally produced SBF file is unofficial. Official candidates require the
private repository's complete Rust and TypeScript suites, two identical clean
SBF builds with an attested host and pinned toolchain, the full external Light
artifact suite, a one-shot receipt, SHA-256 provenance, exact ProgramData
verification, compatible current-state preservation, and governed approval.
