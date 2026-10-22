# Cross-language fixtures

These vectors check that the Rust and TypeScript implementations agree on
account layouts, instruction encoding, PDA derivation, and digests.

To regenerate the ceremony account fixture from the Rust implementation:

```sh
cargo test -p upgrade_controller --lib \
  release1_ceremony_state::tests::regenerate_release1_ceremony_fixture \
  -- --exact --ignored
```

Run the independent consumers from the repository root:

```sh
cargo test -p upgrade_controller --test release1_ceremony_golden
cd clients/ts
node --import tsx --test upgradeGovernance/release1CeremonyGolden.test.ts
```
