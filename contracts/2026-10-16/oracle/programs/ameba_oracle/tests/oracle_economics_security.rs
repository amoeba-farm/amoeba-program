use ameba_oracle::{
    constants::ORACLE_PRE_LISTING_WINDOW_SECONDS,
    instruction::{
        ConfigureOracleEconomicsTemplateV2Params, InitializeOracleMonthV5Params, OracleInstruction,
        OracleInstructionTag,
    },
    state::OracleEconomicParams,
};
use borsh::{BorshDeserialize, BorshSerialize};

#[test]
fn economics_remain_a_versioned_future_month_template() {
    let instruction = OracleInstruction::ConfigureOracleEconomicsTemplateV2 {
        params: ConfigureOracleEconomicsTemplateV2Params {
            expected_config_version: 7,
            economics: OracleEconomicParams::default(),
        },
    };
    let encoded = instruction.try_to_vec().unwrap();
    assert_eq!(
        encoded[0],
        OracleInstructionTag::ConfigureOracleEconomicsTemplateV2 as u8
    );
    assert_eq!(
        OracleInstruction::try_from_slice(&encoded).unwrap(),
        instruction
    );
}

#[test]
fn v5_is_the_only_constructible_month_initializer() {
    let scramble_start_ts = 1_777_478_400;
    let instruction = OracleInstruction::InitializeOracleMonthV5 {
        params: InitializeOracleMonthV5Params {
            scramble_start_ts,
            listing_ts: scramble_start_ts + ORACLE_PRE_LISTING_WINDOW_SECONDS,
            settlement_base_oracle_atomic: 10_000,
            required_sku_root: [7; 32],
            required_sku_count: 52,
        },
    };
    let encoded = instruction.try_to_vec().unwrap();
    assert_eq!(
        encoded[0],
        OracleInstructionTag::InitializeOracleMonthV5 as u8
    );
    assert_eq!(
        OracleInstruction::try_from_slice(&encoded).unwrap(),
        instruction
    );
}
