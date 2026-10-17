# Original-path verification builds

Cargo embeds source-path metadata. This image preserves the original absolute
checkout root while inheriting the pinned Agave 4.0.0 build environment.
The full repository must be mounted at its WORKDIR; workspace paths are relative
to that root. Do not mount only one component or add a legacy `--mount-path`.

The published image is pinned by its immutable registry digest. From the
repository root, use verifier 0.5.2:

```sh
IMAGE='us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67'
solana-verify build . --base-image "$IMAGE" --workspace-path "$PWD/oracle" --library-name ameba_oracle --arch v1 --cargo-build-sbf-args '--tools-version v1.53'
```

Use the default production features. The verifier supplies `--locked`.
Compare both raw artifact SHA-256 and the verifier executable hash with the
installation record before publishing verification metadata. A matching local
build and a successful hosted verification are distinct results; this recipe
alone claims neither. [REPRODUCTION.json](REPRODUCTION.json) records the local
container builds that matched the oracle and previous Futures binaries at that event.
Hosted verification is recorded in
[the completion receipt](../deployment-evidence/mainnet-verification-20261016-completed.json).
Historical Spread recipes and records
remain unchanged.

## Previous Futures AMBA Earn build

The Futures upgrade from source `315cae4d` used the checkout root
`/opt/ameba-dev/worktrees/amba-release`. The preceding gate-final image and
old Futures reproduction record are historical for that program. The Oracle
recipe above remains unchanged.

```sh
FUTURES_IMAGE='us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/amba-release@sha256:36bdcd0005a5a41f9eba76add58f16fccdc7a2b7b2fbace1be3aaba8cda7b40e'
solana-verify build . --base-image "$FUTURES_IMAGE" --workspace-path "$PWD/futures" --library-name ameba_futures --arch v1 --cargo-build-sbf-args '--tools-version v1.53'
```

[Current source and reproduction evidence](../contracts/2026-10-16-amba-earn)
records the exact installed-byte match. A bounded standard compiler path-remap
experiment under the old image still produced a different hash. Updated hosted
verification remains separate and pending.

## Current inventory-netting candidate

The [inventory-netting source and reproduction record](../contracts/2026-10-17-inventory-netting)
uses the same immutable `amba-release` image and Futures command above.
Its two independent builds match exactly at 482,000 bytes. This preserves the
fixed controller recipe while advancing the selected source commit.
The Oracle recipe remains unchanged. The older AMBA Earn reproduction remains
historical; deployment and hosted verification of this candidate are pending.
