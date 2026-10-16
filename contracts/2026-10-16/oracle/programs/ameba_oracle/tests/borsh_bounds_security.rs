use ameba_oracle::{
    constants::{
        CURRENT_STATE_NAMESPACE_SEED, MAX_INSTRUCTION_DATA_BYTES, MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS,
        ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES, USER_COLLATERAL_PDA_SEED,
    },
    error::OracleError,
    instruction::{ConfigureOracleProductSkuManifestParams, SubmitOracleOpeningClaimParams},
    state::{OracleOpeningClaim, UserCollateral},
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};

#[test]
fn processor_rejects_instruction_data_above_global_cap_before_dispatch() {
    let mut at_limit = vec![0u8; MAX_INSTRUCTION_DATA_BYTES];
    at_limit[0] = u8::MAX;
    assert_eq!(
        ameba_oracle::processor::process_instruction(&ameba_oracle::id(), &[], &at_limit),
        Err(ProgramError::Custom(
            OracleError::InvalidInstructionData as u32
        ))
    );

    // Over the cap the size check runs first, even for an assigned tag.
    for tag in [0u8, 1, 190, u8::MAX] {
        let mut data = vec![0u8; MAX_INSTRUCTION_DATA_BYTES + 1];
        data[0] = tag;
        assert_eq!(
            ameba_oracle::processor::process_instruction(&ameba_oracle::id(), &[], &data),
            Err(ProgramError::Custom(
                OracleError::InstructionTooLarge as u32
            )),
            "tag {tag}"
        );
    }
}

mod common;

#[tokio::test]
async fn duplicate_user_collateral_init_preserves_every_live_balance_byte() {
    use solana_sdk::{
        account::{Account, AccountSharedData},
        signature::{Keypair, Signer},
    };
    let mut h = common::start().await;
    let program_id = ameba_oracle::id();
    let user = Keypair::new();
    h.fund(&user.pubkey(), 10_000_000).await;
    let (collateral, bump) = Pubkey::find_program_address(
        &[
            CURRENT_STATE_NAMESPACE_SEED,
            USER_COLLATERAL_PDA_SEED,
            user.pubkey().as_ref(),
        ],
        &program_id,
    );
    let live_state = UserCollateral {
        is_initialized: true,
        bump,
        owner: user.pubkey(),
        available_balance: 444_000,
        position_locked_balance: 222_000,
        last_action_slot: 987_654,
    };
    let mut before = vec![0u8; UserCollateral::LEN];
    let serialized = live_state.try_to_vec().unwrap();
    before[..serialized.len()].copy_from_slice(&serialized);
    h.ctx.set_account(
        &collateral,
        &AccountSharedData::from(Account {
            lamports: 10_000_000,
            data: before.clone(),
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        }),
    );
    let ix = ameba_oracle::instruction::init_user_collateral(&program_id, &user.pubkey());
    common::assert_oracle_error(h.send(ix, &[&user]).await, OracleError::AlreadyInitialized);
    let data = h.account(collateral).await.unwrap().data;
    assert_eq!(data, before);
    assert_eq!(UserCollateral::try_from_slice(&data).unwrap(), live_state);
}

#[test]
fn product_sku_draft_chunk_decoder_is_bounded_before_identifier_allocation() {
    let params = |len| ConfigureOracleProductSkuManifestParams {
        underlying_id: [7; 32],
        draft_nonce: 11,
        expected_start_index: 0,
        sku_count: 52,
        expected_root: [3; 32],
        sku_id_chunk: vec![[9; 32]; len],
    };
    let at_limit = params(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
        .try_to_vec()
        .unwrap();
    assert!(ConfigureOracleProductSkuManifestParams::try_from_slice(&at_limit).is_ok());

    let over_limit = params(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS + 1)
        .try_to_vec()
        .unwrap();
    assert!(ConfigureOracleProductSkuManifestParams::try_from_slice(&over_limit).is_err());

    let mut extreme_prefix = Vec::new();
    extreme_prefix.extend_from_slice(&[7; 32]);
    extreme_prefix.extend_from_slice(&11u64.to_le_bytes());
    extreme_prefix.extend_from_slice(&0u16.to_le_bytes());
    extreme_prefix.extend_from_slice(&52u16.to_le_bytes());
    extreme_prefix.extend_from_slice(&[3; 32]);
    extreme_prefix.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(ConfigureOracleProductSkuManifestParams::try_from_slice(&extreme_prefix).is_err());
}

#[test]
fn opening_url_decoder_accepts_exact_cap_and_rejects_over_cap_and_u32_max() {
    let params = |url: String| SubmitOracleOpeningClaimParams {
        opening_state: 1,
        source_time: 2,
        stake: 3,
        canonical_locator_hash: [4; 32],
        source_definition_hash: [5; 32],
        archive_url: url,
    };
    let at_limit = params("x".repeat(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES))
        .try_to_vec()
        .unwrap();
    assert!(SubmitOracleOpeningClaimParams::try_from_slice(&at_limit).is_ok());
    let over_limit = params("x".repeat(ORACLE_OPENING_ARCHIVE_URL_MAX_BYTES + 1))
        .try_to_vec()
        .unwrap();
    assert!(SubmitOracleOpeningClaimParams::try_from_slice(&over_limit).is_err());

    let mut forged = params(String::new()).try_to_vec().unwrap();
    let length_offset = forged.len() - 4;
    forged[length_offset..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(SubmitOracleOpeningClaimParams::try_from_slice(&forged).is_err());
}

#[test]
fn opening_claim_state_is_fixed_width_and_rejects_truncation() {
    // The validated archive URL is now represented by a fixed bytes32 commitment. The active
    // claim therefore has no attacker-controlled dynamic allocation or length prefix.
    let encoded = OracleOpeningClaim::default().try_to_vec().unwrap();
    assert_eq!(encoded.len(), 144);
    assert!(encoded.len() <= OracleOpeningClaim::LEN);

    let mut exact = encoded.as_slice();
    assert!(OracleOpeningClaim::deserialize(&mut exact).is_ok());
    assert!(exact.is_empty());

    let mut truncated = &encoded[..encoded.len() - 1];
    assert!(OracleOpeningClaim::deserialize(&mut truncated).is_err());
}
