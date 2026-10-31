# Oracle heads upgrade, 2026-10-30

Installed production oracle source for Mainnet program
`45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS`. The files come from release commit
`2af39bf9f69f783bacf929daf9b1baa103ade351`, whose oracle and governance trees are identical
to the qualified source `179864284b5ca09e2ce0bfdb94a05884d2e6f1ee`: the compiled inputs are the same.

This adds Oracle v2 "heads" (instruction tag 210) under the governance envelope and the
BATX Dec 1 cleanup (tag 30, selector 25). The production artifact compiles the v1 monthly
formation out: Mainnet forms no more v1 months, and its Live months keep everything up to
their payout. The only default feature is `heads`; `heads-migrate`, `v1-formation` and the
localnet profiles are not selected. The Futures program is unchanged.

The build uses the existing oracle verification image and fixed checkout path.
ELF size: 726,200 bytes, SBPF v1. SHA-256: `a634450ad1f671cfd5745d3a99cda389fbee5c73dd5f1d4ff55c363047a349d3`.
Verifier executable hash: `86a6f3a8ccff422804a19eedd2e438842dd42c45c511b70e064bf0b28e4abde3`.

`SOURCE_FILES.json` binds every exported file to its original Git blob and SHA-256.
Run `python3 contracts/2026-10-30-oracle-heads/verify-source.py` from the repository root.
The original manifests, lockfiles, production code and build dependencies are unmodified.
`REPRODUCTION.json` records the fixed-path builds and the rebuild of this public source
with the hosted verifier's recipe, which reproduced the exact ELF byte for byte.

Installed at finalized slot **463788239**. Mainnet's loader requires at least 10,240
additional bytes per extension, so the allocation is 727,904 bytes: the exact ELF followed
by 1,704 zero bytes. The normalized verification hash is unchanged by that padding.
Updated hosted verification is pending.

From the repository root, using solana-verify 0.5.2:

```sh
solana-verify build . --base-image us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67 --workspace-path "$PWD/oracle" --library-name ameba_oracle --arch v1 --cargo-build-sbf-args='--tools-version v1.53'
```

No feature flags are added. The verifier supplies `--locked`.
The 2026-10-22 Oracle record verifies its public source `482f01f7774205595b13c83608c2693465420706`
as an immutable snapshot, as the earlier Futures records do.
