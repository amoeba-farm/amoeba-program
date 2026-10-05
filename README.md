# Amoeba Program

Solana smart contracts for Amoeba's index options protocol. The program implements
collateral custody, oracle source selection, contract issuance, settlement, and
an integrated discrete liquidity market maker (DLMM).

[Website](https://amoeba.farm) · [Governance](https://github.com/amoeba-farm/amoeba-governance) · [Security](SECURITY.md)

## Source

The Rust program is in [`programs/light_token_minter`](programs/light_token_minter).

| Path | Contents |
| --- | --- |
| [`src/processor`](programs/light_token_minter/src/processor) | Instruction handlers and account validation |
| [`src/state.rs`](programs/light_token_minter/src/state.rs) | Account types and state modules |
| [`src/instruction.rs`](programs/light_token_minter/src/instruction.rs) | Instruction definitions and codecs |
| [`governance`](governance) | Governed product and benchmark manifests |
| [`deployments`](deployments) | Program identities and deployment configuration |

## Build

Use the pinned Rust toolchain in [`rust-toolchain.toml`](rust-toolchain.toml).
To check the Devnet configuration:

```sh
cargo check --manifest-path programs/light_token_minter/Cargo.toml \
  --locked --lib \
  --features devnet-v3-governance-controller,devnet-solo-backfill-2026
```

## Deployment and verification

This source tree supports Devnet and Mainnet profiles. The selected Mainnet artifact, exact build inputs, and publication, deployment
and verification status are recorded in
[`MAINNET_BUILD.json`](MAINNET_BUILD.json); historical Devnet configuration remains
in [Devnet configuration](deployments/devnet-v3.json).

This is a source preview. The private current release selection and its
qualification bundle are omitted: they include operator evidence and generated
artifacts outside this source export. The preview does not assert that this
source revision is the installed Mainnet program or authorize deployment.

[Source provenance](PUBLIC_SOURCE_PROVENANCE.json) identifies the source revision
and exported file digest. [Historical build verification](deployment-evidence/writer-dlmm-verified-build-20260910.json)
records an earlier artifact. Deployment records describe the revisions they
attest; they do not establish current chain state or gate activation.

## License

See [LICENSE](LICENSE).
