# Inventory-backed quotes and overlap netting

This source candidate exports the exact Futures workspace from release
`release/2026-10-17-inventory-netting`, commit
`b306e314d60bfc1145f88ee54a158cc6694a94da`.
[SOURCE_FILES.json](SOURCE_FILES.json) records all 22 workspace files.
[DEPENDENCY_INPUTS.json](DEPENDENCY_INPUTS.json) records the seven external
oracle/governance interface inputs from the same commit; preserved public bytes
match those source inputs. Oracle source and historical verification records
are unchanged.

The previous [AMBA Earn source record](../2026-10-16-amba-earn) remains immutable.
Its checker uses pinned public commit `8153c2d78938580eb3603c6690e4117908d84cdf`,
so advancing the current Futures source does not rewrite its historical claims.

## Fixed-path production artifact

Two independent container builds produced identical **482,000-byte** ELFs:

- Raw SHA-256: `0215dc5bb67cec367e2386bc8478658e9c65d2ddde20fd20ca2362adc69c3fd0`.
- Verifier-normalized SHA-256: `cfd004d73708961817667ba185af75bbc439b4dfbc3ffd7aa14a26eb4ed7efe6`.

Both use the immutable image
`us-central1-docker.pkg.dev/amoeba-hm0q2k/solana-verifiable-builds/amba-release@sha256:36bdcd0005a5a41f9eba76add58f16fccdc7a2b7b2fbace1be3aaba8cda7b40e`,
checkout root `/opt/ameba-dev/worktrees/amba-release`, workspace `futures`,
default production features, SBPF architecture `v1` and platform tools `v1.53`.
Both builds passed stack-diagnostic and dev-marker scans.
[REPRODUCTION.json](REPRODUCTION.json) records the build inputs and outputs.

The release's earlier native `inn-release` build has a different path-dependent
hash (`c96354b48b0989aba22869b4948e6b79169fbb1daaa6d8a49e8157f8d26b1a27`); this candidate selects the fixed-path artifact above instead.
Source-bound passing release tests are reused. Eight focused tests against this exact
artifact passed in LiteSVM: inventory-backed bids/asks, shifted and partial
overlap netting, claim-slot exhaustion and all three wide-book/compute cases.
[QUALIFICATION.json](QUALIFICATION.json) records the exact artifact and results.

Deployment and updated hosted verification are pending. Publishing this source
or matching two local builds does not claim either has completed.
