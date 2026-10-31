//! The production (mainnet) artifact beside the formation build: `tests/production_artifact.rs`
//! and `tests/production_failed_month.rs` load the formation build (`SBF_OUT_DIR`) as an
//! upgradeable program and upgrade it to the production artifact, as mainnet will take it.
use super::*;

pub fn sbf_elf(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The formation build (`SBF_OUT_DIR`, as every SBF test loads it).
pub fn formation_elf() -> Vec<u8> {
    let dir = std::env::var_os("SBF_OUT_DIR")
        .or_else(|| std::env::var_os("BPF_OUT_DIR"))
        .expect("SBF_OUT_DIR names the test-sbf build");
    sbf_elf(&std::path::Path::new(&dir).join("ameba_oracle.so"))
}

/// The production artifact: `AMEBA_ORACLE_PRODUCTION_SO`, which must exist, or else
/// `oracle/target/deploy/ameba_oracle.so` when it was built (these tests skip without it).
pub fn production_elf() -> Option<Vec<u8>> {
    if let Some(path) = std::env::var_os("AMEBA_ORACLE_PRODUCTION_SO") {
        return Some(sbf_elf(std::path::Path::new(&path)));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/deploy/ameba_oracle.so");
    if !path.exists() {
        eprintln!(
            "skipped: no production artifact at {} (build it, or set AMEBA_ORACLE_PRODUCTION_SO)",
            path.display()
        );
        return None;
    }
    Some(sbf_elf(&path))
}

/// The formation build, upgradeable to the production artifact (`None`: skip).
pub async fn start_formation() -> Option<(Harness, Vec<u8>)> {
    let production = production_elf()?;
    let formation = formation_elf();
    assert_ne!(formation, production, "the two builds are the same file");
    let capacity = formation.len().max(production.len());
    Some((start_upgradeable(&formation, capacity).await, production))
}
