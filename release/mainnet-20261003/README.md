# Mainnet source publication: 2026-10-03

This publication exports canonical source commit
`61d735117c95f131fd1bdb819d9b62b6e21173cb`, used for the Mainnet artifact
`8e9828ec9d20555991813bbc9bde7ee0ca19e300f6114bacb224f7ab4f9f9781`
(1,453,584 bytes). The recorded upgrade slot is 452844697. The governance gate
was Active at epoch 21 in the finalized observation at slot 452876957.

## Build inputs

The reviewed profile is [profile.json](profile.json), with its exact generated
Rust constants in [mainnet-profile.rs](mainnet-profile.rs). From the repository
root on Linux, select that file explicitly:

```sh
export AMEBA_MAINNET_PROFILE_RS="$PWD/release/mainnet-20261003/mainnet-profile.rs"
cargo check --manifest-path programs/light_token_minter/Cargo.toml \
  --locked --lib --features mainnet-v3
```

The canonical artifact used cargo-build-sbf 4.0.0, platform-tools v1.53 and SBF
Rust 1.89.0, architecture v1, the `mainnet-v3` feature, and the empty default
feature. Its SBF command, from `programs/light_token_minter`, was:

```sh
cargo-build-sbf --arch v1 --offline --features mainnet-v3 \
  --sbf-out-dir /tmp/amoeba-mainnet-sbf -- --locked
```

The offline command requires its dependencies to have been fetched. Use the
crate directory so Cargo loads its checked-in target flags.

## Evidence and scope

Two independent canonical-source SBF builds produced identical bytes, and the
installed artifact binding matches the deployed ELF plus its zero padding.
The public export passed deterministic export, locked metadata, formatting and
Mainnet host compilation checks. A reproducible SBF match from this sanitized
public tree and hosted Solana verification have not been established.

[MAINNET_BUILD.json](../../MAINNET_BUILD.json) binds the source, artifact, profile,
ProgramData and gate observations. [publication-inventory.json](publication-inventory.json)
identifies the 210 generated source files separately from the reviewed public
build inputs, evidence and retained historical records. The generated-tree hash
in [PUBLIC_SOURCE_PROVENANCE.json](../../PUBLIC_SOURCE_PROVENANCE.json) covers
the original 210-file canonical export. Its `publicMetadataOverrides` records
the later public CI correction and both workflow hashes. Contract source files
retain their original generated hashes. Provenance and supplemental release
records are outside that generated-tree hash.

The paired build and source-binding receipts retain their original preparation
status fields. The later upgrade and activation observations record deployment.
Qualification describes the retained tests and does not establish current
end-to-end trading or settlement readiness.

Private tests, operator clients, deployment tooling, credentials, generated
binaries and private Git history are excluded. Previous public release records
remain available with their original contents and in Git history.
