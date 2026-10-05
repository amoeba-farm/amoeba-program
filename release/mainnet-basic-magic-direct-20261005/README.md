# Mainnet Basic direct-wallet source and build

This candidate exports committed source `fea3f4b1bba84b1aa06d49442a379fb8ba3fba63`. Two native builds produced
the same 1,758,480-byte ELF, SHA-256 `a8f44ac3016b6caa79a32ed0d6dfdb7c3f391b75998a07fc35ccb55d9c196e92`.
Basic capped-futures strips accept owner-signed wallet funds. Earn allocations
retain the previously installed open-market behavior and accounting checks.

The reviewed profile is [profile.json](profile.json). Cargo uses the checked-in
relative `mainnet-profile.rs` configuration. From the repository root on Linux:

```sh
solana-verify build "$PWD/programs/light_token_minter" \
  --library-name light_token_minter \
  --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 \
  --arch v1 -- --features mainnet-v3
```

Use solana-verify 0.5.1. It supplies `--locked`. The empty default feature remains
enabled. [MAINNET_BUILD.json](../../MAINNET_BUILD.json) records candidate status;
the prior Earn source and recipe remain available in commit `2e31ad38e0e0d4a3953826abbdbe3a60162c7b96`.
[The public Docker build](public-docker-build.json) reproduced both native artifacts byte for byte.
No deployment or hosted verification is claimed by this preparation.
