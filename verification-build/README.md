# Original-path verification builds

Cargo embeds source-path metadata. This image preserves the original absolute
checkout root while inheriting the pinned Agave 4.0.0 build environment.
The full repository must be mounted at its WORKDIR; workspace paths are relative
to that root. Do not mount only one component or add a legacy `--mount-path`.

The published image is pinned by its immutable registry digest. From the
repository root, use verifier 0.5.2:

```sh
IMAGE='us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67'
solana-verify build . --base-image "$IMAGE" --workspace-path "$PWD/futures" --library-name ameba_futures --arch v1 --cargo-build-sbf-args '--tools-version v1.53'
solana-verify build . --base-image "$IMAGE" --workspace-path "$PWD/oracle" --library-name ameba_oracle --arch v1 --cargo-build-sbf-args '--tools-version v1.53'
```

Use the default production features. The verifier supplies `--locked`.
Compare both raw artifact SHA-256 and the verifier executable hash with the
installation record before publishing verification metadata. A matching local
build and a successful hosted verification are distinct results; this recipe
alone claims neither. [REPRODUCTION.json](REPRODUCTION.json) records the local
container builds that exactly match the installed oracle and futures binaries.
Hosted verification is recorded in
[the completion receipt](../deployment-evidence/mainnet-verification-20261016-completed.json).
Historical Spread recipes and records
remain unchanged.
