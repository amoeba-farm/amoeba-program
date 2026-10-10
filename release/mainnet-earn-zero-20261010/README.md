# Earn zero epoch Mainnet program: source and build

Private source `5cea68a23eda9fb2f1620744cd5b8909c2ba516b` and independently retained client source `5cea68a23eda9fb2f1620744cd5b8909c2ba516b`.
Both qualified native builds have raw ELF SHA-256 `edc87424b4709283d52ec36b8cf8a6162d910ff4447e1178c1c101376abc3cf1` (1961288 bytes).

From the repository root, use solana-verify 0.5.1 (which supplies Cargo --locked), default features enabled:

```sh
solana-verify build "$PWD/programs/light_token_minter" --library-name light_token_minter --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 --arch v1 -- --features mainnet-v3
```

Raw ELF SHA-256 measures every file byte. The Solana normalized executable hash is a separate verifier result.
The public Docker build matches both qualified native raw ELFs byte for byte. Deployment and hosted verification are not claimed.
