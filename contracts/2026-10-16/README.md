# Mainnet oracle and futures contracts

This directory records the exact contract source closure from monorepo commit
`4caf83f080f7541b1f741e63871c8d701b398e9b`, the source recorded for the
2026-10-16 Mainnet installation. The source workspaces are now repository-root
siblings: `oracle/`, `futures/` and `governance/`. Historical index-options /
Spread source remains at `programs/light_token_minter/`; its records are preserved.

| Program | Mainnet address | Installed ELF bytes | Installed ELF SHA-256 |
| --- | --- | ---: | --- |
| Oracle | `45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS` | 707424 | `4ebbb73b8c0aaac0e07f74b640325c1bae5f990dff9ce7faff36d0a56e4cabb7` |
| Futures | `Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu` | 420984 | `2f1fc1077b8b77ef28b29994084537fcfd093741bd01944a41fa2ff99ab56f4f` |

Both programs were registered with controller
`8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx`, and their gates were
activated at epoch 2. [MAINNET_INSTALLATION.json](MAINNET_INSTALLATION.json)
records the finalized installation, authorities, program data addresses and
transaction signatures. It is historical evidence, not a current chain-health
probe or a claim about later economic initialization, API, web or CLI releases.

## Source and build paths

The sibling workspace layout preserves all original Cargo path dependencies:

- `oracle/programs/ameba_oracle`: production oracle, its consumer interface and
  public CFM registry/manifests.
- `futures/programs/futures`: production futures program, math and client crates.
- `governance/crates/governance-interface`: shared gate verifier. The governance
  workspace members are retained because Cargo resolves the original workspace.

The original component Cargo manifests, lockfiles, source, tests and toolchain
files are unmodified. [SOURCE_FILES.json](SOURCE_FILES.json) lists each exported
file's original path, size, mode and SHA-256. No operator keys, environment files,
runtime state, API, web or private launch evidence are included.

Build from the repository root on Linux with Agave 4.0.0 and SBF platform tools v1.53:

```sh
(cd oracle && cargo-build-sbf --manifest-path programs/ameba_oracle/Cargo.toml --arch v1 --tools-version v1.53 -- --locked -j 3)
(cd futures && cargo-build-sbf --manifest-path programs/futures/Cargo.toml --arch v1 --tools-version v1.53 -- --locked -j 3)
```

Use the default production feature sets; do not enable test, dev, localnet,
synthetic-controller or tracing features for a production artifact. These
commands build only; they do not deploy, initialize or sign anything.

`python3 contracts/2026-10-16/verify-source.py` (from the repository root) checks the exported file hashes and local Cargo path
closure without building programs or contacting a chain.

## Qualification boundary

The installed ELF hashes above belong to the supplied artifacts built in the
original environment. Cargo source-path metadata affects their bytes: rebuilding
the checkout must use the original absolute build path to reproduce them.
[The container recipe](../../verification-build/README.md) preserves that path;
[the reproduction receipt](../../verification-build/REPRODUCTION.json) records
matching oracle and futures builds using the published immutable image.
Finalized authority-signed metadata and hosted-verifier results are recorded in
[the completion receipt](../../deployment-evidence/mainnet-verification-20261016-completed.json).

The later monorepo launch-tool corrections through
`37af722bb47ffa44a5f14dea9cba6bf25212cb49` change host operations, not these
contract source inputs. They are not silently substituted for the installed
contract source. The CLI publication has its own source and release provenance.
