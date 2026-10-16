use std::{env, fs, path::PathBuf, str::FromStr};
fn main() {
    println!("cargo:rerun-if-env-changed=AMEBA_GOVERNANCE_CONTROLLER_ID");
    let localnet = env::var_os("CARGO_FEATURE_LOCALNET_GOVERNANCE").is_some();
    if localnet && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("solana") {
        assert!(env::var_os("AMEBA_GOVERNANCE_CONTROLLER_ID").is_some(), "SBF localnet governance requires AMEBA_GOVERNANCE_CONTROLLER_ID; fixture identity is host-test-only");
    }
    let bytes = match env::var("AMEBA_GOVERNANCE_CONTROLLER_ID") {
        Ok(value) => solana_pubkey::Pubkey::from_str(&value)
            .expect("AMEBA_GOVERNANCE_CONTROLLER_ID must be a public key")
            .to_bytes(),
        Err(_) => [71; 32],
    };
    if localnet {
        assert_ne!(
            solana_pubkey::Pubkey::new_from_array(bytes).to_string(),
            "8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx",
            "localnet must use a distinct controller identity"
        );
    }
    let source = format!(
        "pub const LOCALNET_CONTROLLER_PROGRAM_ID: Pubkey = Pubkey::new_from_array({bytes:?});\n"
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("localnet_identity.rs"),
        source,
    )
    .unwrap();
}
