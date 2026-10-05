# Mainnet source and reproducible build

This release exports canonical source `85d71cbd5861b9e22dbe992d5603862cd3ddf971`.
The qualified Mainnet ELF is 1,757,448 bytes with SHA-256
`00d87a91975555179d8927d2445efd5b04065e1ac4a750b3bc1b956a3ff1f018`.
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
