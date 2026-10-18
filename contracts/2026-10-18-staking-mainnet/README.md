# Staking Mainnet release

This export contains the exact Futures workspace from
`release/2026-10-18-staking-mainnet`, source commit
`c723db776e9d0c639565d89ac761703731b37fb4`.
[SOURCE_FILES.json](SOURCE_FILES.json) records 23 files, including the production
Meteora/Pyth price reader. [DEPENDENCY_INPUTS.json](DEPENDENCY_INPUTS.json) records
seven external dependency inputs checked against that same commit; their bytes
match the preserved Oracle and governance interface closure.

The prior [inventory-netting record](../2026-10-17-inventory-netting) remains
historical and immutable. Its checker reads the source at public commit
`045090bfade3707f48ba7d1d2d5176300751aa36`.

## Reproduced production artifact

The digest-pinned R6 `amba-release` image reproduced the supplied release artifact
byte for byte at the maintained checkout root
`/opt/ameba-dev/worktrees/amba-release`, workspace `futures`, default production
features, SBPF `v1` and platform tools `v1.53`:

- ELF: **706,416 bytes**.
- SHA-256: `6c38326084c53f560bbdbcc54f2f2441191bcee0a2f915850496d8329a4452cf`.
- Verifier-normalized SHA-256: `d37452966b76f2bbaefbd77daf1a48e30c7e21bdaeecdcf6c35d0dbebe92209d`.

[REPRODUCTION.json](REPRODUCTION.json) records the exact build command, image,
toolchain, source inventories and scans. The source-bound full Futures gate
passed at 2026-10-18 09:42:23 UTC (input key `a5d1d84ae5b393a1`). Eight focused
tests against this exact production ELF passed: production pricing validation,
indebted quote compute, shortfall recovery, redemption and credit-aware unstaking.
The indebted whole-grid quote used **875,058 CU**.
[QUALIFICATION.json](QUALIFICATION.json) records scope, logs and limitations.

This release adds stake-backed maker credit, production price validation,
settlement recovery and claim-exit fixes, including the whole-grid indebted quote
compute improvement. Existing market headers retain their 512-byte layout and
version 1, using former spare fields for credit state. New stake pools are
separate governed operations; publishing or installing this artifact does not
create, fund or enable lending from one.

Mainnet installation and updated hosted verification are pending. Prior
verification receipts attest their own historical artifacts.
