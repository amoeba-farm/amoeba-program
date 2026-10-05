# Amoeba Program

Solana smart contracts for Amoeba's index options protocol. The program implements
collateral custody, oracle source selection, contract issuance, settlement, and
an integrated discrete liquidity market maker (DLMM).

[Website](https://amoeba.farm) Â· [Governance](https://github.com/amoeba-farm/amoeba-governance) Â· [Security](SECURITY.md)

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

The Mainnet program [`2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw`](https://explorer.solana.com/address/2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw) is Solana Verified by OtterSec for deployment slot **453609919**. Both the global status and the upgrade authority status matched the installed executable on 2026-10-05T15:13:22.626Z.

- [Verified immutable source](https://github.com/amoeba-farm/amoeba-program/tree/65e20d96369a2732b3e718c6470ab23880ce0380) (`65e20d96369a2732b3e718c6470ab23880ce0380`).
- [Live verification status](https://verify.osec.io/status/2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw) and [authority verification records](https://verify.osec.io/status-all/2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw).
- [Public deployment attestation](deployment-evidence/basic-magic-direct-mainnet-20261005.json) and [build recipe](MAINNET_BUILD.json).

The verified source is commit `65e20d96369a2732b3e718c6470ab23880ce0380`. Later documentation commits record this verification; they are separate from the immutable source revision OtterSec verified. The live status links show the current verifier records.

Historical [Devnet configuration](deployments/devnet-v3.json) and [earlier build verification](deployment-evidence/writer-dlmm-verified-build-20260910.json) retain their original scope.

## License

See [LICENSE](LICENSE).
