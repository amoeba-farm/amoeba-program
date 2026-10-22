# Oracle upgrade, 2026-10-22

Installed production oracle source from `da0e9143f44635dcbce7288ea6510ddf042de215` for Mainnet program
`45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS`. The compiled inputs are
identical to the approved package source `6dc23b87f54419f595fcd51fa69732180a1d6632`.

This adds authenticated evidence kind 4 and an 80/20 split for new sponsorships.
Existing sponsorships retain their recorded split. The Futures program is unchanged.
Localnet-only instructions are excluded by the production feature selection.

The build uses the existing oracle verification image and fixed checkout path.
ELF size: 710,360 bytes. SHA-256: `cb0a5687b452edbb5fcc7c1474b71b15382c747c0fea30e2f225da0c8251d14f`.
Verifier executable hash: `7c3da6d17741916d47e3c6158c8123f630733faa4b084a6f7ec6b2d366c7f94e`.

`SOURCE_FILES.json` binds every exported file to its original Git blob and SHA-256.
Run `python3 contracts/2026-10-22-oracle-upgrade/verify-source.py` from the repository.
The original manifests, lockfiles, production code and build dependencies are unmodified.
The public source export was rebuilt with solana-verify 0.5.2 and reproduced the
exact ELF byte for byte. Installed at finalized slot **459945306**
and activated at gate epoch **5**. Updated hosted verification passed for public
source `482f01f7774205595b13c83608c2693465420706`, with both canonical and
authority-specific hashes matching the installed program.

Mainnet's active loader rule requires at least 10,240 additional bytes per extension.
The allocation is therefore 717,664 bytes: the exact 710,360-byte ELF followed by
7,304 zero bytes. The normalized verification hash is unchanged by that padding.
The 3.609467 SOL buffer deposit was refunded; the extension used 0.0520192 SOL.
The oracle's frozen program-owned state, Futures code and gate, and controller
code and unrelated accounts were preserved. API configuration is a separate handoff.
See [finalized installation and verification evidence](../../deployment-evidence/mainnet-oracle-20261022.json).

From the repository root, using solana-verify 0.5.2:

```sh
solana-verify build . --base-image us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/gate-final@sha256:7ad1e567f19831e658f5b892a7269d23d7f1826438e025b5d9d2e385240a4d67 --workspace-path "$PWD/oracle" --library-name ameba_oracle --arch v1 --cargo-build-sbf-args='--tools-version v1.53'
```

No feature flags are added. The verifier supplies `--locked`.
Historical Futures verification uses public source `8536d411b2af0bd77b37740edc315db2f06db601`.
Its recorded dependency tree remains reproducible and its on-chain program is unchanged.
