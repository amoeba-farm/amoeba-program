# Mainnet source and reproducible build

This release exports canonical source `f635d38950ad8fdd75a160b8af41bc81359c6a9d`.
The qualified Mainnet ELF is 1,757,320 bytes with SHA-256
`69705f2fcb92a27b9730b7a91984122a66ba4910900f1a098ae63d8c1a0d182a`.
The reviewed profile is [profile.json](profile.json). Its generated Rust file is
also included in the program directory; Cargo selects that file through its
checked-in relative environment configuration.

From the repository root on Linux:

```sh
solana-verify build "$PWD/programs/light_token_minter" \
  --library-name light_token_minter \
  --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 \
  --arch v1 -- --features mainnet-v3
```

The [public Docker build](public-docker-build.json) reproduced both qualified
native production artifacts byte for byte.

Use solana-verify 0.5.1. It supplies `--locked` to Cargo. The selected feature is
`mainnet-v3`, with the empty default feature retained. The program explicitly
rejects the legacy Devnet timing feature when Mainnet is selected.

[MAINNET_BUILD.json](../../MAINNET_BUILD.json) records the exact artifact,
profile and current publication/deployment/verification status. The source
provenance hash covers the maintained sanitized export. The publication inventory
separately records the exact generated profile and self-contained Cargo setup.
Private tests, clients, credentials, deployment tooling and binaries are excluded.
