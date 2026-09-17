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

The program is deployed on **Solana Mainnet Beta and Devnet** at
`2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw`.
The two networks have separate state and build profiles. The
[integration manifest](release/current-integration.json) and
[Devnet configuration](deployments/devnet-v3.json) describe Devnet.

[Mainnet build evidence](MAINNET_BUILD.json) records the deployed artifact and
the exact public build recipe. Its generated profile contains public identities
only; no RPC credentials or signing keys are needed to build.
The Mainnet build enables `mainnet-four-hour-launch` for the September/October
cohorts and rejects the Devnet backfill feature.

With solana-verify 0.5.1, run from this repository's root:

```sh
solana-verify build "$PWD/programs/light_token_minter" \
  --workspace-path "$PWD/programs/light_token_minter" \
  --library-name light_token_minter \
  --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 \
  --arch v1 -- --no-default-features --features mainnet-v3
```

[Source provenance](PUBLIC_SOURCE_PROVENANCE.json) identifies the private source
revision and the public reproducibility additions. The current build evidence
records two matching private-source native SBF builds. This sanitized public
export has not yet been independently rebuilt; public native, Docker and hosted
verification are not claimed. Hosted verification remains a
separate result; check the [verifier status](https://verify.osec.io/status/2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw).
The upgraded Mainnet program's gate is Active at epoch 9. Market and oracle
bootstrap readiness is separate from executable identity.

## License

See [LICENSE](LICENSE).
