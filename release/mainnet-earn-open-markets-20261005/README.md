# Mainnet Earn allocation source and build

This candidate exports committed source `6b10b9d2b05346fd4552f03abfb9791375170ad2`. Two native builds produced
the same 1,756,952-byte ELF, SHA-256 `763f859a79bfddd8c2a4589627c652d99d0443077dff4272a520116f7cb035a8`.
It permits the fund allocator to enter existing eligible markets while retaining
account identity, lifecycle, cash, inventory, and solvency checks.

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
the old resting-order source and recipe remain available in commit `5c303ab5bf76e1e5ad0bfff131870b9e968964b2`.
[The public Docker build](public-docker-build.json) reproduced both native artifacts byte for byte.
No deployment or hosted verification is claimed by this preparation.
