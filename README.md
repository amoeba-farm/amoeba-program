# Amoeba Program

Solana smart contract for Amoeba Farm's capped options protocol. This program is the whole deal, implementing
collateral custody, oracle source selection, contract issuance, settlement, and
a fully integrated discrete liquidity market maker (DLMM).

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

The published deployment is on **Solana Devnet**. Its program ID is
`2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw`.
See the [integration manifest](release/current-integration.json) and
[Devnet configuration](deployments/devnet-v3.json).

[Source provenance](PUBLIC_SOURCE_PROVENANCE.json) identifies the source revision
and exported file digest. [Build verification](deployment-evidence/writer-dlmm-verified-build-20260910.json)
records the published artifact's build configuration and hashes. Deployment
records describe the revisions they attest; they do not establish current chain state.

## License

See [LICENSE](LICENSE).
