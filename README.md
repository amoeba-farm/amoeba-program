# Amoeba Program

Solana smart contracts for Amoeba's oracle and range-futures markets, together
with the preserved source of its earlier index-options / Spread program.

[Website](https://amoeba.farm) · [Governance](https://github.com/amoeba-farm/amoeba-governance) · [Security](SECURITY.md)

The Mainnet oracle and range-futures source is in [oracle](oracle) and
[futures](futures). The [staking release](contracts/2026-10-18-staking-mainnet)
records the installed Futures source `c723db776e9d0c639565d89ac761703731b37fb4` and its exact
reproduced artifact. The [inventory-netting](contracts/2026-10-17-inventory-netting),
[AMBA Earn](contracts/2026-10-16-amba-earn) and
[original](contracts/2026-10-16) records preserve earlier releases.
The earlier index-options / Spread program remains in
[programs/light_token_minter](programs/light_token_minter).

## Mainnet programs

| Program | Address | Verification |
| --- | --- | --- |
| Oracle | `45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS` | [Solana Verified Builds status](https://verify.osec.io/status/45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS) |
| Range futures | `Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu` | [Solana Verified Builds status](https://verify.osec.io/status/Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu) |
| Governance controller | `8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx` | [Solana Verified Builds status](https://verify.osec.io/status/8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx) |

The staking Futures artifact is installed at finalized slot **458402515**
and Active at gate epoch **11**. The exact **706,416-byte** program matches its
reproduced source. Existing market and token custody state was preserved; the
maker and keeper resumed after unsigned runtime checks passed (2 maker messages simulated successfully; 0 keeper messages emitted). The API
remained online. The Mainnet stake pool was then created at finalized slot
**458406056**; no stake or lending funding was performed.

Futures passed updated hosted verification on 2026-10-18 for public source
`8536d411b2af0bd77b37740edc315db2f06db601`, with matching canonical and authority-specific hashes.
The controller remains at release 6; its and the Oracle's verification records
are unchanged. [Installation and verification evidence](deployment-evidence/mainnet-staking-20261018.json) records
the finalized council transactions, artifact hashes, buffer refund and runtime checks.

All three programs passed hosted source verification on 2026-10-16.
[Finalized metadata and hosted-verification evidence](deployment-evidence/mainnet-verification-20261016-completed.json)
records the selected source commits, deployed hashes, authority-signed records
and verifier jobs. Oracle and futures matched their
[published source](https://github.com/amoeba-farm/amoeba-program/tree/624728f222d719a5011e2d92dff0eb50a4c7d6f7)
and the binaries installed at that verification event; neither program required
an upgrade for that event. This historical evidence does not attest the later
Futures AMBA Earn binary.
At that historical event, the controller matched
[release-5 source](https://github.com/amoeba-farm/amoeba-governance/tree/1a8f9e3fa3c80e3d0d1a20651742a0b5e17640fe).
Original source and installation records remain in
[contracts/2026-10-16](contracts/2026-10-16).

The earlier Mainnet Spread program
`2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw` is closed. Its
[historical verification status](https://verify.osec.io/status/2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw)
and source remain available. Mainnet status links do not attest historical
Devnet deployments at the same addresses.

## Source

The current programs are in [`oracle/programs/ameba_oracle`](oracle/programs/ameba_oracle)
and [`futures/programs/futures`](futures/programs/futures). Their reproducible
build recipe is in [verification-build](verification-build).

The historical Spread program is in [`programs/light_token_minter`](programs/light_token_minter).

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

The historical Spread source tree supports Devnet and Mainnet profiles.
[`MAINNET_BUILD.json`](MAINNET_BUILD.json) describes an earlier prepared release
with a pending deployment status; it is not the current futures installation.
Historical Devnet configuration remains in
[Devnet configuration](deployments/devnet-v3.json).

The historical Spread tree is a source preview. Its private release selection and
qualification bundle are omitted: they include operator evidence and generated
artifacts outside this source export. The preview does not assert that this
source revision is the installed Mainnet program or authorize deployment.

[Source provenance](PUBLIC_SOURCE_PROVENANCE.json) identifies the source revision
and exported file digest. [Historical build verification](deployment-evidence/writer-dlmm-verified-build-20260910.json)
records an earlier artifact. Deployment records describe the revisions they
attest; they do not establish current chain state or gate activation.

## License

See [LICENSE](LICENSE).
