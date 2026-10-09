# 923692c Mainnet program: source and build

Private source `923692c717bad8460424122c0093fb399598436b` and independently retained client source `923692c717bad8460424122c0093fb399598436b`.
Both qualified native builds have raw ELF SHA-256 `9afb5fb2f6b5555165e157b7f681e6007e9d5354fd4ec935df4205cc0268c8a2` (1961032 bytes).

From the repository root, use solana-verify 0.5.1 (which supplies Cargo --locked), default features enabled:

```sh
solana-verify build "$PWD/programs/light_token_minter" --library-name light_token_minter --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 --arch v1 -- --features mainnet-v3
```

Raw ELF SHA-256 measures every file byte. The Solana normalized executable hash is a separate verifier result.
The public Docker build matches both qualified native raw ELFs byte for byte. Deployment and hosted verification are not claimed.
