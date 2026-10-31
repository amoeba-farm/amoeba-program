use solana_program::{program_option::COption, pubkey::Pubkey};

use crate::fixed_codec::FixedCursor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountState {
    Initialized,
    Frozen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenAccount {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub delegate: COption<Pubkey>,
    pub state: AccountState,
    pub is_native: COption<u64>,
    pub delegated_amount: u64,
    pub close_authority: COption<Pubkey>,
}

impl TokenAccount {
    pub const LEN: usize = 165;

    pub fn unpack(data: &[u8]) -> Result<Self, ()> {
        unpack_token_account(data).ok_or(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mint {
    pub mint_authority: COption<Pubkey>,
    pub supply: u64,
    pub decimals: u8,
    pub is_initialized: bool,
    pub freeze_authority: COption<Pubkey>,
}

impl Mint {
    pub const LEN: usize = 82;

    pub fn unpack(data: &[u8]) -> Result<Self, ()> {
        unpack_mint(data).ok_or(())
    }
}

#[inline(never)]
fn read_optional_pubkey(input: &mut FixedCursor<'_>) -> COption<Pubkey> {
    let tag = input.u32();
    let value = input.pubkey();
    match tag {
        0 => COption::None,
        1 => COption::Some(value),
        _ => {
            input.invalid = true;
            COption::None
        }
    }
}

#[inline(always)]
fn read_optional_u64(input: &mut FixedCursor<'_>) -> COption<u64> {
    let tag = input.u32();
    let value = input.u64();
    match tag {
        0 => COption::None,
        1 => COption::Some(value),
        _ => {
            input.invalid = true;
            COption::None
        }
    }
}

pub(crate) fn unpack_token_account(data: &[u8]) -> Option<TokenAccount> {
    if data.len() != 165 {
        return None;
    }
    let mut input = FixedCursor::new(data);
    let mint = input.pubkey();
    let owner = input.pubkey();
    let amount = input.u64();
    let delegate = read_optional_pubkey(&mut input);
    let state = match input.u8() {
        1 => AccountState::Initialized,
        2 => AccountState::Frozen,
        _ => return None,
    };
    let value = TokenAccount {
        mint,
        owner,
        amount,
        delegate,
        state,
        is_native: read_optional_u64(&mut input),
        delegated_amount: input.u64(),
        close_authority: read_optional_pubkey(&mut input),
    };
    if input.invalid || input.offset != TokenAccount::LEN {
        return None;
    }
    Some(value)
}

pub(crate) fn unpack_mint(data: &[u8]) -> Option<Mint> {
    if data.len() != 82 || data[45] != 1 {
        return None;
    }
    let mut input = FixedCursor::new(data);
    let mint_authority = read_optional_pubkey(&mut input);
    let supply = input.u64();
    let decimals = input.u8();
    let is_initialized = input.bool();
    let freeze_authority = read_optional_pubkey(&mut input);
    if input.invalid || input.offset != Mint::LEN || !is_initialized {
        return None;
    }
    Some(Mint {
        mint_authority,
        supply,
        decimals,
        is_initialized,
        freeze_authority,
    })
}

/// Token-2022 accounts and mints: the classic base layout followed, when extended, by an account
/// type byte at offset 165 and type-length-value extensions (`spl-token-2022-interface`
/// `extension/mod.rs`). The heads family's AMBA may be a Token-2022 mint; these parsers read
/// exactly what it needs and reject any layout they do not understand.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod token2022 {
    use super::*;

    /// Extension type numbers (`ExtensionType`), the ones this program names.
    pub const TRANSFER_FEE_CONFIG: u16 = 1;
    pub const TRANSFER_FEE_AMOUNT: u16 = 2;
    pub const IMMUTABLE_OWNER: u16 = 7;
    pub const NON_TRANSFERABLE: u16 = 9;
    pub const PERMANENT_DELEGATE: u16 = 12;
    pub const TRANSFER_HOOK: u16 = 14;
    pub const PAUSABLE: u16 = 26;
    /// The highest extension type the parser accepts (`PausableAccount`); later ones reject.
    pub const LAST_KNOWN_EXTENSION: u16 = 27;

    /// A multisig's length: never an extensible account (Token-2022 refuses it).
    const MULTISIG_LEN: usize = 355;
    const ACCOUNT_TYPE_OFFSET: usize = TokenAccount::LEN;
    const TLV_START: usize = ACCOUNT_TYPE_OFFSET + 1;
    const ACCOUNT_TYPE_MINT: u8 = 1;
    const ACCOUNT_TYPE_ACCOUNT: u8 = 2;

    /// One transfer fee schedule.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct TransferFee {
        pub epoch: u64,
        pub maximum_fee: u64,
        pub basis_points: u16,
    }

    impl TransferFee {
        /// Token-2022's fee on `amount`: `ceil(amount * bps / 10000)`, at most `maximum_fee`.
        pub fn fee(&self, amount: u64) -> u64 {
            if self.basis_points == 0 || amount == 0 {
                return 0;
            }
            let numerator = u128::from(amount) * u128::from(self.basis_points);
            let fee = numerator.div_ceil(10_000);
            u64::try_from(fee).unwrap_or(u64::MAX).min(self.maximum_fee)
        }
    }

    /// The mint's `TransferFeeConfig` extension.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct TransferFeeConfig {
        pub config_authority: Option<Pubkey>,
        pub withdraw_withheld_authority: Option<Pubkey>,
        pub withheld_amount: u64,
        pub older: TransferFee,
        pub newer: TransferFee,
    }

    impl TransferFeeConfig {
        pub const LEN: usize = 108;

        /// The schedule in force at `epoch`.
        pub fn at_epoch(&self, epoch: u64) -> &TransferFee {
            if epoch >= self.newer.epoch {
                &self.newer
            } else {
                &self.older
            }
        }
    }

    /// A parsed mint and what its extensions say.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ExtendedMint {
        pub base: Mint,
        pub extensions: Vec<u16>,
        pub transfer_fee: Option<TransferFeeConfig>,
    }

    impl ExtendedMint {
        /// Whether a vault of this mint is safe custody: no extension lets anyone but the
        /// vault's owner move or freeze its balance, or makes transfers out depend on another
        /// program (a permanent delegate, a transfer hook, a pause authority, non-transferable
        /// tokens). A protection against theft and lost funds, not a policy choice.
        pub fn is_custody_safe(&self) -> bool {
            !self.extensions.iter().any(|t| {
                matches!(
                    *t,
                    NON_TRANSFERABLE | PERMANENT_DELEGATE | TRANSFER_HOOK | PAUSABLE
                )
            })
        }
    }

    /// A parsed token account and the extension types it carries.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ExtendedTokenAccount {
        pub base: TokenAccount,
        pub extensions: Vec<u16>,
    }

    /// The `(type, value)` entries of a TLV region, as Token-2022 walks it: it ends at a zero
    /// type or when fewer than two bytes remain; a truncated length or value, or a type this
    /// parser does not know, rejects.
    pub fn tlv_entries(tlv: &[u8]) -> Option<Vec<(u16, &[u8])>> {
        let mut entries = Vec::new();
        let mut start = 0usize;
        while start < tlv.len() {
            if tlv.len() - start < 2 {
                break;
            }
            let kind = u16::from_le_bytes([tlv[start], tlv[start + 1]]);
            if kind == 0 {
                break;
            }
            if kind > LAST_KNOWN_EXTENSION || tlv.len() - start < 4 {
                return None;
            }
            let len = usize::from(u16::from_le_bytes([tlv[start + 2], tlv[start + 3]]));
            let value_start = start + 4;
            let value_end = value_start.checked_add(len)?;
            if value_end > tlv.len() {
                return None;
            }
            entries.push((kind, &tlv[value_start..value_end]));
            start = value_end;
        }
        Some(entries)
    }

    fn optional_nonzero_pubkey(bytes: &[u8]) -> Option<Pubkey> {
        let key = Pubkey::new_from_array(bytes.try_into().ok()?);
        (!crate::pubkey_is_default(&key)).then_some(key)
    }

    fn transfer_fee(bytes: &[u8]) -> TransferFee {
        let u64_at = |o: usize| u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap_or([0; 8]));
        TransferFee {
            epoch: u64_at(0),
            maximum_fee: u64_at(8),
            basis_points: u16::from_le_bytes([bytes[16], bytes[17]]),
        }
    }

    fn transfer_fee_config(value: &[u8]) -> Option<TransferFeeConfig> {
        if value.len() != TransferFeeConfig::LEN {
            return None;
        }
        Some(TransferFeeConfig {
            config_authority: optional_nonzero_pubkey(&value[0..32]),
            withdraw_withheld_authority: optional_nonzero_pubkey(&value[32..64]),
            withheld_amount: u64::from_le_bytes(value[64..72].try_into().ok()?),
            older: transfer_fee(&value[72..90]),
            newer: transfer_fee(&value[90..108]),
        })
    }

    /// The TLV region of an extended account or mint whose base occupies `base_len` bytes, after
    /// checking the zero padding up to the account type byte and that byte itself.
    fn extended_tlv(data: &[u8], base_len: usize, account_type: u8) -> Option<&[u8]> {
        if data.len() == MULTISIG_LEN || data.len() < TLV_START {
            return None;
        }
        if !crate::bytes_are_zero(&data[base_len..ACCOUNT_TYPE_OFFSET])
            || data[ACCOUNT_TYPE_OFFSET] != account_type
        {
            return None;
        }
        Some(&data[TLV_START..])
    }

    /// A mint under `token_program`: a classic mint is exactly 82 bytes; a Token-2022 mint is 82
    /// bytes or extended.
    pub fn unpack_mint_any(token_program: &Pubkey, data: &[u8]) -> Option<ExtendedMint> {
        if crate::token_instruction::check_id(token_program) || data.len() == Mint::LEN {
            return Some(ExtendedMint {
                base: unpack_mint(data)?,
                extensions: Vec::new(),
                transfer_fee: None,
            });
        }
        if !crate::pubkey_eq(
            token_program,
            &crate::token_instruction::TOKEN_2022_PROGRAM_ID,
        ) {
            return None;
        }
        let base = unpack_mint(data.get(..Mint::LEN)?)?;
        let entries = tlv_entries(extended_tlv(data, Mint::LEN, ACCOUNT_TYPE_MINT)?)?;
        let mut transfer_fee = None;
        for (kind, value) in &entries {
            if *kind == TRANSFER_FEE_CONFIG {
                transfer_fee = Some(transfer_fee_config(value)?);
            }
        }
        Some(ExtendedMint {
            base,
            extensions: entries.iter().map(|(kind, _)| *kind).collect(),
            transfer_fee,
        })
    }

    /// A token account under `token_program`: a classic account is exactly 165 bytes; a
    /// Token-2022 account is 165 bytes or extended.
    pub fn unpack_token_account_any(
        token_program: &Pubkey,
        data: &[u8],
    ) -> Option<ExtendedTokenAccount> {
        if crate::token_instruction::check_id(token_program) || data.len() == TokenAccount::LEN {
            return Some(ExtendedTokenAccount {
                base: unpack_token_account(data)?,
                extensions: Vec::new(),
            });
        }
        if !crate::pubkey_eq(
            token_program,
            &crate::token_instruction::TOKEN_2022_PROGRAM_ID,
        ) {
            return None;
        }
        let base = unpack_token_account(data.get(..TokenAccount::LEN)?)?;
        let tlv = extended_tlv(data, TokenAccount::LEN, ACCOUNT_TYPE_ACCOUNT)?;
        Some(ExtendedTokenAccount {
            base,
            extensions: tlv_entries(tlv)?.iter().map(|(kind, _)| *kind).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::program_pack::Pack;
    use spl_token::state::{
        Account as SplTokenAccount, AccountState as SplAccountState, Mint as SplMint,
    };

    fn assert_mint_acceptance_matches_spl(data: &[u8]) {
        assert_eq!(
            Mint::unpack(data).is_ok(),
            <SplMint as Pack>::unpack(data).is_ok()
        );
    }

    fn assert_token_acceptance_matches_spl(data: &[u8]) {
        assert_eq!(
            TokenAccount::unpack(data).is_ok(),
            <SplTokenAccount as Pack>::unpack(data).is_ok()
        );
    }

    #[test]
    fn fixed_mint_codec_matches_spl_pack() {
        let authority = Pubkey::new_unique();
        let freeze = Pubkey::new_unique();
        for mint_authority in [COption::None, COption::Some(authority)] {
            for freeze_authority in [COption::None, COption::Some(freeze)] {
                let expected = SplMint {
                    mint_authority,
                    supply: 91,
                    decimals: 6,
                    is_initialized: true,
                    freeze_authority,
                };
                let mut data = [0_u8; SplMint::LEN];
                SplMint::pack(expected, &mut data).unwrap();
                let decoded = Mint::unpack(&data).unwrap();
                assert_eq!(decoded.mint_authority, mint_authority);
                assert_eq!(decoded.supply, 91);
                assert_eq!(decoded.decimals, 6);
                assert!(decoded.is_initialized);
                assert_eq!(decoded.freeze_authority, freeze_authority);
                assert_mint_acceptance_matches_spl(&data);
            }
        }

        let mut data = [0_u8; SplMint::LEN];
        SplMint::pack(
            SplMint {
                mint_authority: COption::None,
                supply: 91,
                decimals: 6,
                is_initialized: false,
                freeze_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        assert_mint_acceptance_matches_spl(&data);
        assert_mint_acceptance_matches_spl(&data[..data.len() - 1]);
        let mut oversized = data.to_vec();
        oversized.push(0);
        assert_mint_acceptance_matches_spl(&oversized);
        for tag_offset in [0, 46] {
            data[45] = 1;
            data[tag_offset..tag_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
            assert_mint_acceptance_matches_spl(&data);
            data[tag_offset..tag_offset + 4].copy_from_slice(&0_u32.to_le_bytes());
        }
        data[45] = 2;
        assert_mint_acceptance_matches_spl(&data);
    }

    #[test]
    fn fixed_token_codec_matches_spl_pack() {
        let mint = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let delegate = Pubkey::new_unique();
        let close_authority = Pubkey::new_unique();
        for delegate_option in [COption::None, COption::Some(delegate)] {
            for state in [
                (SplAccountState::Initialized, AccountState::Initialized),
                (SplAccountState::Frozen, AccountState::Frozen),
            ] {
                for is_native in [COption::None, COption::Some(11)] {
                    for close_option in [COption::None, COption::Some(close_authority)] {
                        let expected = SplTokenAccount {
                            mint,
                            owner,
                            amount: 73,
                            delegate: delegate_option,
                            state: state.0,
                            is_native,
                            delegated_amount: 7,
                            close_authority: close_option,
                        };
                        let mut data = [0_u8; SplTokenAccount::LEN];
                        SplTokenAccount::pack(expected, &mut data).unwrap();
                        let decoded = TokenAccount::unpack(&data).unwrap();
                        assert_eq!(decoded.mint, mint);
                        assert_eq!(decoded.owner, owner);
                        assert_eq!(decoded.amount, 73);
                        assert_eq!(decoded.delegate, delegate_option);
                        assert_eq!(decoded.state, state.1);
                        assert_eq!(decoded.is_native, is_native);
                        assert_eq!(decoded.delegated_amount, 7);
                        assert_eq!(decoded.close_authority, close_option);
                        assert_token_acceptance_matches_spl(&data);
                    }
                }
            }
        }

        let mut data = [0_u8; SplTokenAccount::LEN];
        SplTokenAccount::pack(
            SplTokenAccount {
                mint,
                owner,
                amount: 73,
                delegate: COption::None,
                state: SplAccountState::Uninitialized,
                is_native: COption::None,
                delegated_amount: 7,
                close_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        assert_token_acceptance_matches_spl(&data);
        assert_token_acceptance_matches_spl(&data[..data.len() - 1]);
        let mut oversized = data.to_vec();
        oversized.push(0);
        assert_token_acceptance_matches_spl(&oversized);
        data[108] = 3;
        assert_token_acceptance_matches_spl(&data);
        data[108] = 1;
        for tag_offset in [72, 109, 129] {
            data[tag_offset..tag_offset + 4].copy_from_slice(&2_u32.to_le_bytes());
            assert_token_acceptance_matches_spl(&data);
            data[tag_offset..tag_offset + 4].copy_from_slice(&0_u32.to_le_bytes());
        }
    }

    mod token2022_parsing {
        use super::super::token2022::*;
        use super::*;
        use crate::token_instruction::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};

        fn tlv(kind: u16, value: &[u8]) -> Vec<u8> {
            let mut out = kind.to_le_bytes().to_vec();
            out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            out.extend_from_slice(value);
            out
        }

        fn base_mint(decimals: u8) -> Vec<u8> {
            let mut data = vec![0u8; SplMint::LEN];
            SplMint::pack(
                SplMint {
                    mint_authority: COption::Some(Pubkey::new_unique()),
                    supply: 1_000,
                    decimals,
                    is_initialized: true,
                    freeze_authority: COption::None,
                },
                &mut data,
            )
            .unwrap();
            data
        }

        fn base_account(mint: Pubkey, owner: Pubkey) -> Vec<u8> {
            let mut data = vec![0u8; SplTokenAccount::LEN];
            SplTokenAccount::pack(
                SplTokenAccount {
                    mint,
                    owner,
                    amount: 55,
                    delegate: COption::None,
                    state: SplAccountState::Initialized,
                    is_native: COption::None,
                    delegated_amount: 0,
                    close_authority: COption::None,
                },
                &mut data,
            )
            .unwrap();
            data
        }

        fn extended_mint(extensions: &[u8]) -> Vec<u8> {
            let mut data = base_mint(9);
            data.resize(165, 0);
            data.push(1);
            data.extend_from_slice(extensions);
            data
        }

        fn fee_config_value(bps: u16, maximum: u64) -> Vec<u8> {
            let mut value = vec![0u8; 32];
            value.extend_from_slice(&[7u8; 32]);
            value.extend_from_slice(&3u64.to_le_bytes());
            for epoch in [0u64, 9] {
                value.extend_from_slice(&epoch.to_le_bytes());
                value.extend_from_slice(&maximum.to_le_bytes());
                value.extend_from_slice(&bps.to_le_bytes());
            }
            value
        }

        #[test]
        fn classic_layouts_parse_exactly_as_before() {
            let mint = base_mint(6);
            let parsed = unpack_mint_any(&TOKEN_PROGRAM_ID, &mint).unwrap();
            assert_eq!(parsed.base.decimals, 6);
            assert!(parsed.extensions.is_empty() && parsed.is_custody_safe());
            let mut longer = mint.clone();
            longer.resize(200, 0);
            assert!(unpack_mint_any(&TOKEN_PROGRAM_ID, &longer).is_none());
            let account = base_account(Pubkey::new_unique(), Pubkey::new_unique());
            assert_eq!(
                unpack_token_account_any(&TOKEN_PROGRAM_ID, &account)
                    .unwrap()
                    .base
                    .amount,
                55
            );
            assert!(unpack_mint_any(&Pubkey::new_unique(), &longer).is_none());
        }

        #[test]
        fn token_2022_mint_with_a_transfer_fee_parses_its_schedule() {
            let data = extended_mint(
                &[
                    tlv(TRANSFER_FEE_CONFIG, &fee_config_value(25, u64::MAX)),
                    tlv(18, &[1u8; 64]),
                ]
                .concat(),
            );
            let parsed = unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &data).unwrap();
            assert_eq!(parsed.base.decimals, 9);
            assert_eq!(parsed.extensions, vec![TRANSFER_FEE_CONFIG, 18]);
            assert!(parsed.is_custody_safe());
            let fee = parsed.transfer_fee.unwrap();
            assert_eq!(fee.config_authority, None);
            assert_eq!(
                fee.withdraw_withheld_authority,
                Some(Pubkey::new_from_array([7; 32]))
            );
            assert_eq!(fee.withheld_amount, 3);
            assert_eq!(fee.at_epoch(9).basis_points, 25);
            // 25 bp of 10 AMBA, rounded up; the cap binds.
            assert_eq!(fee.at_epoch(9).fee(10_000_000_000), 25_000_000);
            assert_eq!(fee.at_epoch(9).fee(1), 1);
            assert_eq!(fee.at_epoch(9).fee(0), 0);
            let capped = TransferFee {
                epoch: 0,
                maximum_fee: 5,
                basis_points: 25,
            };
            assert_eq!(capped.fee(10_000_000_000), 5);
            // An unextended Token-2022 mint is the classic layout.
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &base_mint(9)).is_some());
            // A trailing zero type ends the walk, as in Token-2022.
            let mut padded = data.clone();
            padded.extend_from_slice(&[0, 0, 9, 9]);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &padded).is_some());
            // One leftover byte cannot hold a type and ends the walk too.
            let mut odd = data.clone();
            odd.push(5);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &odd).is_some());
        }

        #[test]
        fn unsafe_custody_extensions_are_reported() {
            for kind in [
                NON_TRANSFERABLE,
                PERMANENT_DELEGATE,
                TRANSFER_HOOK,
                PAUSABLE,
            ] {
                let value = vec![0u8; if kind == NON_TRANSFERABLE { 0 } else { 32 }];
                let data = extended_mint(&tlv(kind, &value));
                let parsed = unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &data).unwrap();
                assert!(!parsed.is_custody_safe(), "extension {kind}");
            }
        }

        #[test]
        fn malformed_extended_layouts_reject() {
            let good = extended_mint(&tlv(TRANSFER_FEE_CONFIG, &fee_config_value(25, 9)));
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &good).is_some());
            // Wrong account type, nonzero padding, a multisig length, too short to extend.
            let mut wrong_type = good.clone();
            wrong_type[165] = 2;
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &wrong_type).is_none());
            let mut dirty = good.clone();
            dirty[100] = 1;
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &dirty).is_none());
            let mut multisig = good.clone();
            multisig.resize(355, 0);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &multisig).is_none());
            let mut short = base_mint(9);
            short.resize(120, 0);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &short).is_none());
            // A length past the end, a truncated length, an unknown type, a bad fee length.
            let mut overrun = extended_mint(&[]);
            overrun.extend_from_slice(&[1, 0, 200, 0, 1, 2]);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &overrun).is_none());
            let mut truncated = extended_mint(&[]);
            truncated.extend_from_slice(&[1, 0, 4]);
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &truncated).is_none());
            let unknown = extended_mint(&tlv(LAST_KNOWN_EXTENSION + 1, &[]));
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &unknown).is_none());
            let bad_fee = extended_mint(&tlv(TRANSFER_FEE_CONFIG, &[0u8; 107]));
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &bad_fee).is_none());
            // An uninitialized base rejects.
            let mut uninitialized = good.clone();
            uninitialized[45] = 0;
            assert!(unpack_mint_any(&TOKEN_2022_PROGRAM_ID, &uninitialized).is_none());
        }

        #[test]
        fn token_2022_accounts_parse_their_base_and_extension_types() {
            let mint = Pubkey::new_unique();
            let owner = Pubkey::new_unique();
            let mut data = base_account(mint, owner);
            assert!(unpack_token_account_any(&TOKEN_2022_PROGRAM_ID, &data).is_some());
            data.push(2);
            data.extend_from_slice(&tlv(TRANSFER_FEE_AMOUNT, &4u64.to_le_bytes()));
            data.extend_from_slice(&tlv(IMMUTABLE_OWNER, &[]));
            let parsed = unpack_token_account_any(&TOKEN_2022_PROGRAM_ID, &data).unwrap();
            assert_eq!(parsed.base.mint, mint);
            assert_eq!(parsed.base.owner, owner);
            assert_eq!(parsed.base.amount, 55);
            assert_eq!(
                parsed.extensions,
                vec![TRANSFER_FEE_AMOUNT, IMMUTABLE_OWNER]
            );
            // The classic program never accepts an extended account; a mint type byte rejects.
            assert!(unpack_token_account_any(&TOKEN_PROGRAM_ID, &data).is_none());
            let mut mint_typed = data.clone();
            mint_typed[165] = 1;
            assert!(unpack_token_account_any(&TOKEN_2022_PROGRAM_ID, &mint_typed).is_none());
            let mut multisig = data.clone();
            multisig.resize(355, 0);
            assert!(unpack_token_account_any(&TOKEN_2022_PROGRAM_ID, &multisig).is_none());
        }
    }
}
