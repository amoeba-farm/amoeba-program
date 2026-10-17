# Futures AMBA Earn source

The repository-root Futures workspace exports exact monorepo source
`315cae4d2ab23d0a54dd7a6717503349bb3fd9c4`.
[SOURCE_FILES.json](SOURCE_FILES.json) records its 22 files.
[DEPENDENCY_INPUTS.json](DEPENDENCY_INPUTS.json) records the external oracle and
governance-interface crate inputs from that same commit. Their preserved public
source bytes match those inputs exactly.

The original [2026-10-16 inventory](../2026-10-16/SOURCE_FILES.json) and receipts
remain historical. Its checker validates pinned public source
`624728f222d719a5011e2d92dff0eb50a4c7d6f7`, rather than treating this later
Futures revision as the earlier installed program.

The installed artifact contains **444,816 bytes**, with raw SHA-256
`b0480f9adc17c4d58a3f78bc84cb235c6b7ad92782ce6b00bbbf7c55fc666873`
and verifier-normalized hash
`bb068e2a37216b8ba0aced762a69d51424bd35253629dfac5c7379cf42c1a075`.
Its original checkout root was `/opt/ameba-dev/worktrees/amba-release`.
The build ran from `futures/`, using default production features, architecture
`v1` and platform tools `v1.53`:

```sh
cargo-build-sbf --manifest-path programs/futures/Cargo.toml \
  --arch v1 --tools-version v1.53 -- --locked
```

Updated hosted verification is pending. Earlier verification records attest
the previous Futures binary. Oracle source and its historical verification
records remain unchanged.

## Exact matching-path reproduction

The [container recipe](../../verification-build/Dockerfile.futures-315cae4d)
preserves the original checkout root. Its published immutable image is
`us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/amba-release@sha256:36bdcd0005a5a41f9eba76add58f16fccdc7a2b7b2fbace1be3aaba8cda7b40e`.
[REPRODUCTION.json](REPRODUCTION.json) records the measured exact installed-byte
match and the mismatch under the previous fixed build path. These container
results are separate from updated hosted verification and authority metadata.
