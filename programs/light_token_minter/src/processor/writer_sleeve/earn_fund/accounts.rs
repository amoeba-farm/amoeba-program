//! Exact loaders and create-only constructors for Earn Fund accounts.
use super::*;
use crate::compact_error::CompactAccountInfo;

pub(super) fn fund_error(error: FundMathError) -> ProgramError {
    match error {
        FundMathError::Overflow => VaultError::ArithmeticOverflow,
        FundMathError::InvalidAmount => VaultError::EarnFundInvalidAmount,
        FundMathError::InsufficientCash
        | FundMathError::InstantCapExceeded
        | FundMathError::Invested => VaultError::EarnFundInstantUnavailable,
        FundMathError::NotReady => VaultError::EarnFundNotReady,
        FundMathError::BudgetExceeded => VaultError::EarnFundInvalidAllocation,
        FundMathError::QueueBusy => VaultError::EarnFundQueueBusy,
        FundMathError::InvalidPrice | FundMathError::InvalidState => VaultError::EarnFundAccounting,
    }
    .into()
}

/// The singleton fund: owner, exact length, canonical PDA and stored bump,
/// layout, canonical vault identity, consistent queue and slot aggregate,
/// and bounded parameters.
#[inline(never)]
pub(in crate::processor::writer_sleeve) fn load_fund(
    program: &Pubkey,
    info: &AccountInfo,
) -> Result<Box<EarnFundV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<EarnFundV1>(
        info,
        program,
        EarnFundV1::LEN,
        VaultError::EarnFundInvalidAccount,
    )?);
    let (key, bump) = derive_earn_fund(program);
    let (vault, vault_bump) = derive_earn_fund_vault(program, info.key);
    if *info.key != key
        || !value.has_current_layout()
        || value.bump != bump
        || value.usdc_vault != vault
        || value.usdc_vault_bump != vault_bump
        || crate::pubkey_is_default(&value.vault_config)
        || crate::pubkey_is_default(&value.usdc_mint)
        || value.ledger().is_none()
        || !value.params().is_valid()
    {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    Ok(value)
}

/// The fund's slot of `sleeve`: owner, exact length, layout, the sleeve it
/// names, its canonical PDA by the stored bump, open principal, writable.
#[inline(never)]
pub(in crate::processor::writer_sleeve) fn load_slot(
    program: &Pubkey,
    info: &AccountInfo,
    sleeve: &Pubkey,
) -> Result<EarnFundSlotV1, ProgramError> {
    let value = if info.owner == program && info.is_writable {
        EarnFundSlotV1::read(&info.try_data()?)
    } else {
        None
    };
    match value {
        Some(value)
            if value.sleeve == *sleeve
                && value.principal != 0
                && value.mark.state <= crate::earn_fund_math::SLOT_FINAL
                && Pubkey::create_program_address(
                    &[
                        CURRENT_STATE_NAMESPACE_SEED,
                        EARN_FUND_SLOT_SEED,
                        sleeve.as_ref(),
                        &[value.bump()],
                    ],
                    program,
                )
                .is_ok_and(|key| key == *info.key) =>
        {
            Ok(value)
        }
        _ => Err(VaultError::EarnFundInvalidAccount.into()),
    }
}

/// The fund's classic SPL vault: canonical key, mint, PDA authority, no
/// delegate or close authority.
pub(super) fn fund_vault_balance(
    fund_info: &AccountInfo,
    fund: &EarnFundV1,
    vault_info: &AccountInfo,
) -> Result<u64, ProgramError> {
    if *vault_info.key != fund.usdc_vault || !vault_info.is_writable {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    Ok(validate_vault_token_account(vault_info, &fund.usdc_mint, fund_info.key)?.amount)
}

/// Every bucket must stay physically backed after any instruction.
pub(super) fn require_backed(fund: &EarnFundV1, vault_info: &AccountInfo) -> ProgramResult {
    let ledger = fund.ledger().ok_or(VaultError::EarnFundAccounting)?;
    if validate_token_account(vault_info)?.amount
        < ledger.required_vault_atoms().map_err(fund_error)?
    {
        return Err(VaultError::EarnFundAccounting.into());
    }
    Ok(())
}

pub(super) fn fund_signer_seeds(bump: &[u8; 1]) -> [&[u8]; 3] {
    [CURRENT_STATE_NAMESPACE_SEED, EARN_FUND_SEED, bump]
}

/// One investor position for the duration of an instruction: its canonical
/// compressed address, the client's witness and the decoded state.
pub(super) struct PositionSession {
    address: [u8; 32],
    seed: light_sdk::address::AddressSeed,
    witness: EarnFundPositionWitness,
    old_data: Vec<u8>,
    pub position: EarnFundPositionV1,
}

impl PositionSession {
    /// The packed Light-block index of the output queue this instruction
    /// writes to (the position's and every compressed USDC output's).
    pub(super) fn output_queue_index(&self) -> u8 {
        match self.witness {
            EarnFundPositionWitness::New {
                output_state_tree_index,
                ..
            }
            | EarnFundPositionWitness::Live {
                output_state_tree_index,
                ..
            }
            | EarnFundPositionWitness::Closed {
                output_state_tree_index,
                ..
            } => output_state_tree_index,
        }
    }
}

/// Open `owner`'s position from its witness. Only Deposit may create a new
/// address or reopen a closed one; every other caller needs a live account.
/// The supplied state is authenticated by Light when the transition consumes
/// the input whose hash commits to these exact bytes at this address.
pub(super) fn open_position(
    program: &Pubkey,
    fund: &Pubkey,
    owner: &Pubkey,
    witness: &EarnFundPositionWitness,
    allow_new: bool,
) -> Result<PositionSession, ProgramError> {
    if crate::pubkey_is_default(owner) {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    let (address, seed) = derive_earn_fund_position_address(program, fund, owner);
    let (position, old_data) = match witness {
        EarnFundPositionWitness::New { .. } | EarnFundPositionWitness::Closed { .. } => {
            if !allow_new {
                return Err(VaultError::EarnFundInvalidAccount.into());
            }
            (
                EarnFundPositionV1::new(*fund, *owner, &PositionLedger::default()),
                Vec::new(),
            )
        }
        EarnFundPositionWitness::Live { state, .. } => {
            let position = EarnFundPositionV1::new(*fund, *owner, state);
            if position.is_empty()
                || (state.queue_shares == 0) != (state.queue_batch_id == 0)
                || (state.queue_shares == 0 && state.queue_paid_atoms != 0)
                || (state.pending_atoms == 0 && state.pending_epoch != 0)
            {
                return Err(VaultError::EarnFundInvalidAccount.into());
            }
            let data = borsh::to_vec(&position).map_err(|_| VaultError::EarnFundAccounting)?;
            (position, data)
        }
    };
    Ok(PositionSession {
        address,
        seed,
        witness: *witness,
        old_data,
        position,
    })
}

/// Write the position back: update, close to Light's empty placeholder once
/// nothing is left, or create/reopen it. `light` is the Light system account
/// block (system program, CPI authority, registered program, compression
/// authority/program, system, then packed trees and queues).
pub(super) fn commit_position<'a>(
    fee_payer: &AccountInfo<'a>,
    light: &[AccountInfo<'a>],
    proof: Option<[u8; 128]>,
    session: PositionSession,
) -> ProgramResult {
    use crate::compression::{apply_program_leaf_transition, ProgramLeafPrior};
    use light_sdk::LightDiscriminator;
    if light.len() < 7 || !fee_payer.is_signer || !fee_payer.is_writable {
        return Err(VaultError::InvalidAccountList.into());
    }
    let new_data = if session.position.is_empty() {
        None
    } else {
        Some(borsh::to_vec(&session.position).map_err(|_| VaultError::EarnFundAccounting)?)
    };
    let output = session.witness.output();
    let meta = session.witness.meta(session.address);
    let prior = match (&session.witness, output.as_ref(), meta.as_ref()) {
        (EarnFundPositionWitness::New { .. }, Some(output), _) => ProgramLeafPrior::New(output),
        (EarnFundPositionWitness::Closed { .. }, _, Some(meta)) => ProgramLeafPrior::Closed(meta),
        (EarnFundPositionWitness::Live { .. }, _, Some(meta)) => {
            ProgramLeafPrior::Live(meta, &session.old_data)
        }
        _ => return Err(VaultError::InvalidCompressionWitness.into()),
    };
    apply_program_leaf_transition(
        fee_payer,
        light,
        proof,
        EarnFundPositionV1::LIGHT_DISCRIMINATOR,
        session.address,
        session.seed,
        prior,
        new_data,
    )
}

/// A record of a closed epoch: canonical PDA by its stored epoch, fund-bound.
#[inline(never)]
pub(super) fn load_fund_epoch(
    program: &Pubkey,
    info: &AccountInfo,
    fund: &Pubkey,
) -> Result<Box<EarnFundEpochV1>, ProgramError> {
    let value = Box::new(load_exact_zero_padded_state::<EarnFundEpochV1>(
        info,
        program,
        EarnFundEpochV1::LEN,
        VaultError::EarnFundInvalidAccount,
    )?);
    let (key, bump) = derive_earn_fund_epoch(program, value.epoch);
    if *info.key != key
        || !value.has_current_layout()
        || value.bump != bump
        || value.fund != *fund
        || usize::from(value.completed_count) > MAX_COMPLETED_BATCHES
        || !info.is_writable
    {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    Ok(value)
}

/// Create the record of a closed epoch.
pub(super) fn create_fund_epoch<'a>(
    program: &Pubkey,
    payer: &AccountInfo<'a>,
    info: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    fund: &Pubkey,
    outcome: &RollOutcome,
    roll_ts: u64,
) -> ProgramResult {
    let (key, bump) = derive_earn_fund_epoch(program, outcome.closed_epoch);
    if *info.key != key || !crate::is_system_program(system.key) || !payer.is_writable {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    validate_create_only_program_account_target(program, info)?;
    create_program_account(
        payer,
        info,
        system,
        program,
        EarnFundEpochV1::LEN,
        &[
            EARN_FUND_EPOCH_SEED,
            &outcome.closed_epoch.to_le_bytes(),
            &[bump],
        ],
    )?;
    store_state(
        info,
        &EarnFundEpochV1::from_roll(*fund, bump, outcome, roll_ts),
    )
}

/// Apply everything a position owes: the conversion of its pending deposit
/// (record of the converting roll at `pending_info`) and its redemption batch
/// (open batch in the fund, or the record of the completing roll at
/// `queue_info`). Each slot is the system program when nothing needs it; the
/// two may name the same record.
pub(super) fn settle_position(
    program: &Pubkey,
    fund_key: &Pubkey,
    ledger: &mut FundLedger,
    position: &mut EarnFundPositionV1,
    pending_info: &AccountInfo,
    queue_info: &AccountInfo,
) -> ProgramResult {
    let mut owed = position.ledger();
    let pending_owed = owed.pending_owed(ledger);
    let queue_state = owed.queue_state(ledger).map_err(fund_error)?;
    let queue_completed = queue_state == QueueState::Completed;
    let sentinel = |info: &AccountInfo| crate::is_system_program(info.key);
    if pending_owed == sentinel(pending_info) || queue_completed == sentinel(queue_info) {
        return Err(VaultError::InvalidAccountList.into());
    }
    let shared = pending_owed && queue_completed && pending_info.key == queue_info.key;
    let mut pending_record = if pending_owed {
        Some(load_fund_epoch(program, pending_info, fund_key)?)
    } else {
        None
    };
    let mut queue_record = if queue_completed && !shared {
        Some(load_fund_epoch(program, queue_info, fund_key)?)
    } else {
        None
    };
    if let Some(record) = pending_record.as_mut() {
        let mut remaining = record.ledger();
        owed.settle_pending(&mut remaining).map_err(fund_error)?;
        record.set_ledger(&remaining);
    }
    match queue_state {
        QueueState::Open(index) => owed.settle_open_batch(ledger, index).map_err(fund_error)?,
        QueueState::Completed => {
            let record = if shared {
                pending_record.as_mut()
            } else {
                queue_record.as_mut()
            }
            .ok_or(VaultError::InvalidAccountList)?;
            let mut remaining = record.ledger();
            owed.settle_completed_batch(&mut remaining, ledger)
                .map_err(fund_error)?;
            record.set_ledger(&remaining);
        }
        QueueState::Empty | QueueState::Accumulating => {}
    }
    position.set_ledger(&owed);
    if let Some(record) = pending_record {
        store_state(pending_info, record.as_ref())?;
    }
    if let Some(record) = queue_record {
        store_state(queue_info, record.as_ref())?;
    }
    Ok(())
}

/// Validate a classic SPL USDC account of `mint` owned by `owner` (any owner
/// when `None`).
pub(super) fn usdc_account(
    info: &AccountInfo,
    mint: &Pubkey,
    owner: Option<&Pubkey>,
) -> Result<TokenAccount, ProgramError> {
    let token = validate_token_account(info)?;
    if token.mint != *mint
        || owner.is_some_and(|owner| token.owner != *owner)
        || token.state != AccountState::Initialized
        || !info.is_writable
    {
        return Err(VaultError::InvalidTokenAccount.into());
    }
    Ok(token)
}

/// Owner-signed classic transfer with exact balance deltas.
pub(super) fn pay_from_owner<'a>(
    token_program: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    amount: u64,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    if source.key == destination.key {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    let source_before = validate_token_account(source)?.amount;
    let destination_before = validate_token_account(destination)?.amount;
    invoke_token_transfer_checked(
        token_program,
        source,
        mint,
        destination,
        owner,
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        &[],
    )?;
    if source_before.checked_sub(validate_token_account(source)?.amount) != Some(amount)
        || validate_token_account(destination)?
            .amount
            .checked_sub(destination_before)
            != Some(amount)
    {
        return Err(VaultError::EarnFundAccounting.into());
    }
    Ok(())
}

/// The cash rails of a position instruction: the Light Token accounts that
/// move USDC between the fund's classic vault and wallet-owned compressed
/// leaves, and the Light system block that also carries the position.
pub(super) struct Rails<'x, 'a> {
    pub payer: &'x AccountInfo<'a>,
    pub mint: &'x AccountInfo<'a>,
    pub spl_token: &'x AccountInfo<'a>,
    pub light_token: &'x AccountInfo<'a>,
    pub token_cpi_authority: &'x AccountInfo<'a>,
    pub spl_interface: &'x AccountInfo<'a>,
    /// Light system program, this program's CPI authority, registered
    /// program, compression authority, compression program, system program,
    /// then the packed trees and queues.
    pub light: &'x [AccountInfo<'a>],
}

impl<'x, 'a> Rails<'x, 'a> {
    /// `fixed` = mint, SPL Token, Light Token, Light Token CPI authority, SPL
    /// interface pool; `light` = the Light block.
    pub(super) fn new(
        payer: &'x AccountInfo<'a>,
        fixed: &'x [AccountInfo<'a>],
        light: &'x [AccountInfo<'a>],
        usdc_mint: &Pubkey,
    ) -> Result<Self, ProgramError> {
        let [mint, spl_token, light_token, token_cpi_authority, spl_interface] = fixed else {
            return Err(VaultError::InvalidAccountList.into());
        };
        if *mint.key != *usdc_mint
            || !crate::token_instruction::check_id(spl_token.key)
            || !crate::light_token_instruction::is_program(light_token.key)
            || !crate::light_token_instruction::is_cpi_authority(token_cpi_authority.key)
            || !spl_interface.is_writable
            || !payer.is_signer
            || !payer.is_writable
            || light.len() < 7
            || !crate::light_token_instruction::is_light_system_program(light[0].key)
            || !crate::light_token_instruction::is_registered_program(light[2].key)
            || !crate::light_token_instruction::is_compression_authority(light[3].key)
            || !crate::light_token_instruction::is_compression_program(light[4].key)
            || !crate::is_system_program(light[5].key)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        validate_spl_interface_account(mint.key, spl_interface)?;
        Ok(Self {
            payer,
            mint,
            spl_token,
            light_token,
            token_cpi_authority,
            spl_interface,
            light,
        })
    }

    fn packed(&self, index: u8) -> Result<&'x AccountInfo<'a>, ProgramError> {
        self.light
            .get(6 + usize::from(index))
            .ok_or_else(|| VaultError::InvalidAccountList.into())
    }
}

/// Fund-signed payout of `amount` from the classic vault into one compressed
/// leaf owned by `recipient` (the claim path's `compress_to_wallet`), with an
/// exact vault delta. `output_queue` is a packed index of the Light block.
pub(super) fn pay_compressed<'a>(
    rails: &Rails<'_, 'a>,
    vault: &AccountInfo<'a>,
    fund_info: &AccountInfo<'a>,
    fund_bump: u8,
    recipient: &AccountInfo<'a>,
    output_queue: u8,
    amount: u64,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    if crate::pubkey_is_default(recipient.key) || recipient.executable {
        return Err(VaultError::EarnFundInvalidAccount.into());
    }
    let queue = rails.packed(output_queue)?;
    let before = validate_token_account(vault)?.amount;
    let ix = light_token_instruction::compress_to_wallet(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        vault.key,
        vault.owner,
        fund_info.key,
        rails.payer.key,
        rails.mint.key,
        recipient.key,
        rails.spl_interface.key,
        [
            rails.light[0].key,
            rails.light[2].key,
            rails.light[3].key,
            rails.light[4].key,
            queue.key,
        ],
    )?;
    let infos = [
        rails.light[0].clone(),
        rails.payer.clone(),
        rails.token_cpi_authority.clone(),
        rails.light[2].clone(),
        rails.light[3].clone(),
        rails.light[4].clone(),
        rails.light[5].clone(),
        queue.clone(),
        rails.mint.clone(),
        vault.clone(),
        fund_info.clone(),
        recipient.clone(),
        rails.spl_interface.clone(),
        rails.spl_token.clone(),
        rails.light_token.clone(),
    ];
    let bump = [fund_bump];
    invoke_signed(&ix, &infos, &[&fund_signer_seeds(&bump)])?;
    if before.checked_sub(validate_token_account(vault)?.amount) != Some(amount) {
        return Err(VaultError::EarnFundAccounting.into());
    }
    Ok(())
}

/// One whole wallet-owned compressed USDC leaf spent by a deposit.
#[derive(Clone, Copy, Debug)]
pub(super) struct CashInput {
    pub amount: u64,
    pub leaf_index: u32,
    pub root_index: u16,
    pub prove_by_index: bool,
    pub tree_index: u8,
    pub queue_index: u8,
    pub output_queue_index: u8,
    pub proof: Option<[u8; 128]>,
}

/// Owner-signed deposit from a compressed leaf: `deposit` is decompressed
/// into the fund's classic vault (the Collect path's Transfer2 encoder) and
/// the change returns to the owner as a compressed leaf. Exact vault delta.
pub(super) fn deposit_compressed<'a>(
    rails: &Rails<'_, 'a>,
    vault: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    cash: CashInput,
    deposit: u64,
) -> ProgramResult {
    use crate::regular_compressed_transfer::{self as transfer, InputLeaf, OutputLeaf};
    use solana_program::instruction::AccountMeta;
    let change = cash
        .amount
        .checked_sub(deposit)
        .ok_or(VaultError::EarnFundInvalidAmount)?;
    if (cash.prove_by_index && cash.root_index != 0)
        || (!cash.prove_by_index && cash.proof.is_none())
        || !owner.is_signer
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let (queue, tree, input_queue) = (
        rails.packed(cash.output_queue_index)?,
        rails.packed(cash.tree_index)?,
        rails.packed(cash.queue_index)?,
    );
    // Light Token's fixed seven metas, then output queue, input tree and
    // queue, mint, owner (input authority and change owner), fund vault, SPL
    // interface pool and SPL Token.
    let infos = [
        rails.light[0].clone(),
        rails.payer.clone(),
        rails.token_cpi_authority.clone(),
        rails.light[2].clone(),
        rails.light[3].clone(),
        rails.light[4].clone(),
        rails.light[5].clone(),
        queue.clone(),
        tree.clone(),
        input_queue.clone(),
        rails.mint.clone(),
        owner.clone(),
        vault.clone(),
        rails.spl_interface.clone(),
        rails.spl_token.clone(),
        rails.light_token.clone(),
    ];
    let metas = infos[..15]
        .iter()
        .enumerate()
        .map(|(i, info)| AccountMeta {
            pubkey: *info.key,
            is_writable: matches!(i, 1 | 7 | 8 | 9 | 12 | 13),
            is_signer: i == 1 || i == 11,
        })
        .collect();
    let (_, interface_bump) =
        light_token_instruction::get_spl_interface_pda_and_bump(rails.mint.key);
    let outputs = [OutputLeaf {
        owner: 4,
        amount: change,
        has_delegate: false,
        delegate: 0,
        mint: 3,
    }];
    let ix = transfer::decompress_to_spl_instruction(
        *rails.light_token.key,
        metas,
        0,
        cash.proof,
        &[InputLeaf {
            owner: 4,
            amount: cash.amount,
            has_delegate: false,
            delegate: 0,
            mint: 3,
            tree: 1,
            queue: 2,
            leaf_index: cash.leaf_index,
            prove_by_index: cash.prove_by_index,
            root_index: cash.root_index,
        }],
        transfer::SplDecompression {
            amount: deposit,
            mint: 3,
            recipient: 5,
            pool_account_index: 6,
            pool_index: 0,
            bump: interface_bump,
            decimals: MarketMintAccounting::CANONICAL_DECIMALS,
        },
        &outputs[..usize::from(change != 0)],
    )?;
    let before = validate_token_account(vault)?.amount;
    invoke_signed(&ix, &infos, &[])?;
    if validate_token_account(vault)?.amount.checked_sub(before) != Some(deposit) {
        return Err(VaultError::EarnFundAccounting.into());
    }
    Ok(())
}
