# Operator-assessed credit, 2026-10-28

Deployed Mainnet source for the Futures program `Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu`.
The registered vault operator can approve an exposure calculated from its private
backend policy. Approval binds the current market snapshot and the exact action.
The program independently checks the signer, vault identity, accounting, custody
and credit limit. The policy's selected interval is not an instruction parameter.
This is a trusted operator assessment, not a proof of a private calculation.
Public positions and approved exposures can still support inference about policy.
Ordinary order fills and withdrawals remain available without an operator approval.

An optional 16-byte approval trailer preserves the existing market record offsets.
Readers must accept that trailer before this upgrade is activated. Older accounts
remain readable and acquire the trailer only when the operator first approves them.

Source commit: `bcf0601606617fda7e00a818e7663d031339858f`. Artifact: 722,520 bytes, SBPF v1.
SHA-256: `42118b8193d5e57c886348ef05272456251e970ac944226f7d70a5eb7e4926ec`.
Normalized executable hash: `b1fd059b10df9b70e57227686372009a8b347a543ebde7b76ff22e01d6791e29`.
Installed at finalized slot `462346311` on 2026-10-28; the governance gate is Active
at epoch 14. Hosted verification passed for public source commit
`059bd727fb61b68adf7d5ee3067ed5158674cbbc`. See `DEPLOYMENT.json` for the deployment,
activation and source-verification receipts. The build inputs and reproduction
record remain unchanged.

`SOURCE_FILES.json` and `DEPENDENCY_INPUTS.json` bind the unmodified build inputs.
Run `python3 contracts/2026-10-28-private-credit/verify-source.py` from the repository root.
Historical release checks continue to verify their immutable source snapshots.

Build with the digest-pinned image and fixed path in `REPRODUCTION.json`, setting
`RUSTUP_TOOLCHAIN=1.93.1-x86_64-unknown-linux-gnu`, then run the recorded command
from the fixed root's `futures` directory. No development feature flags are added.
