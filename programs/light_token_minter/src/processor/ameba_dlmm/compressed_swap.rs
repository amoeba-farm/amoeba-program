use super::*;
use crate::compact_error::CompactAccountInfo;
use crate::{
    ameba_dlmm_instruction::{
        CompressedSwapLeafWitnessV1, SwapCollectiveCompressedExactInV1Params,
    },
    compressed_custody::{derive_compressed_custody, CustodyKind},
    compressed_swap_plan::{self, Direction, Plan},
    regular_compressed_transfer::{self, InputLeaf, OutputLeaf},
};
use solana_program::instruction::AccountMeta;

const FIXED: usize = collective::COLLECTIVE_SWAP_FIXED_ACCOUNTS;
const TAIL: usize = 9;

pub(super) fn existing_sidecar<'a, 'info>(
    program: &Pubkey,
    info: &'a AccountInfo<'info>,
) -> Result<Option<&'a AccountInfo<'info>>, ProgramError> {
    if info.owner == program {
        return Ok(Some(info));
    }
    if crate::is_system_program(info.owner) && info.data_len() == 0 && !info.executable {
        return Ok(None);
    }
    Err(VaultError::InvalidAccountList.into())
}

pub(super) struct CompressedSwapAccounts<'a, 'info> {
    pub(super) after_pool: core::cell::Cell<Option<(u64, u64)>>,
    pub(super) trading_owner: Option<Pubkey>,
    pub(super) base: &'a [AccountInfo<'info>],
    pub(super) pool_custody: &'a AccountInfo<'info>,
    pub(super) writer_cash_custody: &'a AccountInfo<'info>,
    pub(super) user_delegate: &'a AccountInfo<'info>,
    pub(super) retirement: &'a AccountInfo<'info>,
    pub(super) light_system: &'a AccountInfo<'info>,
    pub(super) registered: &'a AccountInfo<'info>,
    pub(super) compression_authority: &'a AccountInfo<'info>,
    pub(super) compression_program: &'a AccountInfo<'info>,
    pub(super) sponsor: &'a AccountInfo<'info>,
    pub(super) merkle: &'a [AccountInfo<'info>],
}

impl<'a, 'info> CompressedSwapAccounts<'a, 'info> {
    pub(super) fn merkle(&self, index: u8) -> Result<&'a AccountInfo<'info>, ProgramError> {
        self.merkle
            .get(usize::from(index))
            .ok_or(VaultError::InvalidAccountList.into())
    }
}

fn valid_context(witness: &CompressedSwapLeafWitnessV1, merkle_count: usize) -> bool {
    usize::from(witness.tree_index) < merkle_count
        && usize::from(witness.queue_index) < merkle_count
        && witness.tree_index != witness.queue_index
}

pub(super) fn parse<'a, 'info>(
    program: &Pubkey,
    a: &'a [AccountInfo<'info>],
    params: &SwapCollectiveCompressedExactInV1Params,
) -> Result<CompressedSwapAccounts<'a, 'info>, ProgramError> {
    let page_count = usize::from(params.page_count);
    let merkle_count = usize::from(params.merkle_account_count);
    if page_count > usize::from(MAX_AMOEBA_DLMM_PAGE_HOPS_PER_SWAP)
        || merkle_count < 2
        || a.len() != FIXED + page_count + TAIL + merkle_count
        || a.len() > 255
        || !valid_context(&params.user_input, merkle_count)
        || params
            .pool_option_input
            .as_ref()
            .is_some_and(|w| !valid_context(w, merkle_count))
        || params
            .pool_quote_input
            .as_ref()
            .is_some_and(|w| !valid_context(w, merkle_count))
        || params
            .writer_quote_input
            .as_ref()
            .is_some_and(|w| !valid_context(w, merkle_count))
        || usize::from(params.output_tree_index) >= merkle_count
        || usize::from(params.output_queue_index) >= merkle_count
        || params.output_tree_index == params.output_queue_index
        || params.user_input_amount == 0
        || params.sponsor_fee_atoms != 10_000
        || (params.swap.direction
            == crate::ameba_dlmm_instruction::AmoebaDlmmSwapDirection::QuoteForOption
            && params.user_input_amount
                < params
                    .swap
                    .amount_in
                    .saturating_add(params.sponsor_fee_atoms))
        || (params.proof.is_none()
            && [
                Some(&params.user_input),
                params.pool_option_input.as_ref(),
                params.pool_quote_input.as_ref(),
                params.writer_quote_input.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|w| !w.prove_by_index))
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let base = &a[..FIXED + page_count];
    validate_collective_swap_base_privileges(program, base)?;
    let tail = &a[FIXED + page_count..];
    let merkle = &tail[TAIL..];
    let scope = crate::scoped_settlement::derive_collective_settlement_delegate(
        program, a[0].key, a[9].key,
    )
    .0;
    let input_scope_valid = *tail[2].key == scope
        || (tail[2].owner == program
            && !tail[2].executable
            && <crate::multi_order::Order as borsh::BorshDeserialize>::try_from_slice(
                &tail[2].try_data()?,
            )
            .ok()
            .and_then(|order| {
                order.settlement_scope(
                    program,
                    tail[2].key,
                    a[0].key,
                    a[2].key,
                    a[9].key,
                    params.user_input_amount,
                )
            })
            .is_some());
    if *tail[0].key != derive_compressed_custody(program, CustodyKind::Pool, a[7].key).0
        || *tail[1].key != derive_compressed_custody(program, CustodyKind::WriterCash, a[27].key).0
        || !tail[0].is_writable
        || !tail[1].is_writable
        || tail[0].is_signer
        || tail[1].is_signer
        || (params.user_input_has_delegate && crate::is_system_program(tail[2].key))
        || (!params.user_input_has_delegate && !crate::is_system_program(tail[2].key))
        || (params.user_input_has_delegate
            && (params.swap.direction
                != crate::ameba_dlmm_instruction::AmoebaDlmmSwapDirection::OptionForQuote
                || !input_scope_valid))
        || tail[2].is_writable
        || tail[2].is_signer
        || *a[23].key != scope
        || *tail[3].key
            != crate::compressed_option_settlement::retirement_owner(program, a[4].key, a[2].key)
        || tail[3].is_writable
        || tail[3].is_signer
        || !crate::light_token_instruction::is_light_system_program(tail[4].key)
        || !crate::light_token_instruction::is_registered_program(tail[5].key)
        || !crate::light_token_instruction::is_compression_authority(tail[6].key)
        || !crate::light_token_instruction::is_compression_program(tail[7].key)
        || tail[4..8]
            .iter()
            .any(|info| info.is_writable || info.is_signer)
        || !tail[8].is_writable
        || !tail[8].is_signer
        || crate::pubkey_is_default(tail[8].key)
        || merkle
            .iter()
            .any(|info| !info.is_writable || info.is_signer || info.executable)
        || merkle.iter().enumerate().any(|(i, info)| {
            merkle[..i]
                .iter()
                .any(|prior| crate::pubkey_eq(prior.key, info.key))
        })
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    Ok(CompressedSwapAccounts {
        after_pool: core::cell::Cell::new(None),
        trading_owner: None,
        base,
        pool_custody: &tail[0],
        writer_cash_custody: &tail[1],
        user_delegate: &tail[2],
        retirement: &tail[3],
        light_system: &tail[4],
        registered: &tail[5],
        compression_authority: &tail[6],
        compression_program: &tail[7],
        sponsor: &tail[8],
        merkle,
    })
}

fn input(
    witness: CompressedSwapLeafWitnessV1,
    owner: u8,
    amount: u64,
    mint: u8,
    delegate: Option<u8>,
) -> InputLeaf {
    InputLeaf {
        owner,
        amount,
        has_delegate: delegate.is_some(),
        delegate: delegate.unwrap_or(0),
        mint,
        tree: witness.tree_index,
        queue: witness.queue_index,
        leaf_index: witness.leaf_index,
        prove_by_index: witness.prove_by_index,
        root_index: witness.root_index,
    }
}

fn output(owner: u8, amount: u64, mint: u8, delegate: Option<u8>) -> OutputLeaf {
    OutputLeaf {
        owner,
        amount,
        has_delegate: delegate.is_some(),
        delegate: delegate.unwrap_or(0),
        mint,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn settle<'a>(
    program: &Pubkey,
    context: &CompressedSwapAccounts<'_, 'a>,
    params: &SwapCollectiveCompressedExactInV1Params,
    pool: &AmoebaDlmmPoolV1,
    route: &crate::writer_dlmm_quote::WriterDlmmRouteQuote,
    direction: AmoebaDlmmSwapDirection,
    authority_seeds: &[&[u8]],
) -> Result<Plan, ProgramError> {
    let a = context.base;
    let pool_info = &a[7];
    let trader = &a[0];
    let payer = context.sponsor;
    let input_mint = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        &a[10]
    } else {
        &a[9]
    };
    let output_mint = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        &a[9]
    } else {
        &a[10]
    };
    let output_vault = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        &a[11]
    } else {
        &a[12]
    };
    let output_interface = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        &a[17]
    } else {
        &a[18]
    };
    let resident = resident_for_pool(program, pool_info, a)?;
    let pool_before = if let Some((_, state)) = &resident {
        Some(pool_ledger(
            program,
            pool_info.key,
            pool,
            state.pool_option,
            state.pool_quote,
        ))
    } else {
        crate::compressed_custody::load(
            program,
            existing_sidecar(program, context.pool_custody)?,
            CustodyKind::Pool,
            pool_info.key,
            &pool.option_mint,
            &pool.quote_mint,
        )?
    }
    .unwrap_or_else(|| {
        let (_, bump) = derive_compressed_custody(program, CustodyKind::Pool, pool_info.key);
        crate::compressed_custody::CompressedCustodyV1::new(
            CustodyKind::Pool,
            *pool_info.key,
            pool.option_mint,
            pool.quote_mint,
            bump,
        )
    });
    let cash_before = crate::compressed_custody::load(
        program,
        existing_sidecar(program, context.writer_cash_custody)?,
        CustodyKind::WriterCash,
        a[27].key,
        &Pubkey::default(),
        &pool.quote_mint,
    )?
    .unwrap_or_else(|| {
        let (_, bump) = derive_compressed_custody(program, CustodyKind::WriterCash, a[27].key);
        crate::compressed_custody::CompressedCustodyV1::new(
            CustodyKind::WriterCash,
            *a[27].key,
            Pubkey::default(),
            pool.quote_mint,
            bump,
        )
    });
    if cash_before.option_atoms != 0 {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    let writer_premium = if direction == AmoebaDlmmSwapDirection::QuoteForOption {
        route
            .writer
            .gross_premium_atoms
            .checked_add(route.writer.lp_fee_atoms)
            .ok_or(VaultError::ArithmeticOverflow)?
    } else {
        0
    };
    let writer_retirement = if direction == AmoebaDlmmSwapDirection::OptionForQuote {
        route.writer.retired_option_atoms
    } else {
        0
    };
    let mut split = compressed_swap_plan::plan(
        if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            Direction::QuoteForOption
        } else {
            Direction::OptionForQuote
        },
        params.user_input_amount,
        route.quote.amount_in,
        route.quote.amount_out,
        writer_premium,
        writer_retirement,
        load_pool_vault(
            program,
            pool_info.key,
            a[8].key,
            &a[8],
            output_mint.key,
            output_vault.key,
            output_vault,
        )?
        .amount,
        pool_before.option_atoms,
        pool_before.quote_atoms,
        cash_before.quote_atoms,
    )?;
    let buy = direction == AmoebaDlmmSwapDirection::QuoteForOption;
    if buy {
        split.user_change = split
            .user_change
            .checked_sub(params.sponsor_fee_atoms)
            .ok_or(VaultError::InvalidAccountList)?;
    } else if route
        .quote
        .amount_out
        .checked_sub(params.sponsor_fee_atoms)
        .filter(|net| *net >= params.swap.minimum_amount_out)
        .is_none()
    {
        return Err(VaultError::InvalidAmoebaDlmmRoute.into());
    }
    let sponsor_from_compressed = if buy {
        0
    } else {
        split
            .output_from_compressed_pool
            .min(params.sponsor_fee_atoms)
    };
    let sponsor_from_hot = if buy {
        0
    } else {
        params.sponsor_fee_atoms - sponsor_from_compressed
    };
    let book_cold = resident
        .as_ref()
        .map(|(_, s)| (s.book_option, s.book_quote))
        .unwrap_or((0, 0));
    let aggregate_before = (
        pool_before
            .option_atoms
            .checked_add(book_cold.0)
            .ok_or(VaultError::ArithmeticOverflow)?,
        pool_before
            .quote_atoms
            .checked_add(book_cold.1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let aggregate_after = (
        split
            .pool_option_after
            .checked_add(book_cold.0)
            .ok_or(VaultError::ArithmeticOverflow)?,
        split
            .pool_quote_after
            .checked_add(book_cold.1)
            .ok_or(VaultError::ArithmeticOverflow)?,
    );
    let custody_info = resident
        .as_ref()
        .map(|(market, _)| *market)
        .unwrap_or(context.pool_custody);
    let use_pool_option = aggregate_before.0 > 0
        && (direction == AmoebaDlmmSwapDirection::OptionForQuote
            || split.output_from_compressed_pool > 0);
    let use_pool_quote = aggregate_before.1 > 0
        && (direction == AmoebaDlmmSwapDirection::QuoteForOption
            || split.output_from_compressed_pool > 0);
    let use_writer_quote = writer_premium > 0 && cash_before.quote_atoms > 0;
    if params.pool_option_input.is_some() != use_pool_option
        || params.pool_quote_input.is_some() != use_pool_quote
        || params.writer_quote_input.is_some() != use_writer_quote
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    if resident.is_none()
        && (split.pool_option_after != pool_before.option_atoms
            || split.pool_quote_after != pool_before.quote_atoms)
    {
        load_or_create_custody(
            program,
            payer,
            context.pool_custody,
            &a[20],
            CustodyKind::Pool,
            pool_info.key,
            &pool.option_mint,
            &pool.quote_mint,
        )?;
    }
    if writer_premium > 0 {
        load_or_create_custody(
            program,
            payer,
            context.writer_cash_custody,
            &a[20],
            CustodyKind::WriterCash,
            a[27].key,
            &Pubkey::default(),
            &pool.quote_mint,
        )?;
    }
    let merkle_count =
        u8::try_from(context.merkle.len()).map_err(|_| VaultError::InvalidAccountList)?;
    let wallet = merkle_count;
    let option = wallet
        .checked_add(1)
        .ok_or(VaultError::InvalidAccountList)?;
    let quote = wallet
        .checked_add(2)
        .ok_or(VaultError::InvalidAccountList)?;
    let pool_owner = wallet
        .checked_add(3)
        .ok_or(VaultError::InvalidAccountList)?;
    let cash_owner = wallet
        .checked_add(4)
        .ok_or(VaultError::InvalidAccountList)?;
    let retirement = wallet
        .checked_add(5)
        .ok_or(VaultError::InvalidAccountList)?;
    let user_delegate = wallet
        .checked_add(6)
        .ok_or(VaultError::InvalidAccountList)?;
    let scope = wallet
        .checked_add(7)
        .ok_or(VaultError::InvalidAccountList)?;
    let mut inputs = Vec::with_capacity(4);
    let mut outputs = Vec::with_capacity(6);
    inputs.push(input(
        params.user_input,
        wallet,
        params.user_input_amount,
        if direction == AmoebaDlmmSwapDirection::QuoteForOption {
            quote
        } else {
            option
        },
        params.user_input_has_delegate.then_some(user_delegate),
    ));
    if let Some(witness) = params.pool_option_input {
        inputs.push(input(witness, pool_owner, aggregate_before.0, option, None));
    }
    if let Some(witness) = params.pool_quote_input {
        inputs.push(input(witness, pool_owner, aggregate_before.1, quote, None));
    }
    if let Some(witness) = params.writer_quote_input {
        inputs.push(input(
            witness,
            cash_owner,
            cash_before.quote_atoms,
            quote,
            None,
        ));
    }
    if aggregate_after.0 > 0
        && (direction == AmoebaDlmmSwapDirection::OptionForQuote
            || split.output_from_compressed_pool > 0)
    {
        outputs.push(output(pool_owner, aggregate_after.0, option, None));
    }
    if aggregate_after.1 > 0
        && (direction == AmoebaDlmmSwapDirection::QuoteForOption
            || split.output_from_compressed_pool > 0)
    {
        outputs.push(output(pool_owner, aggregate_after.1, quote, None));
    }
    if writer_premium > 0 {
        outputs.push(output(cash_owner, split.writer_quote_after, quote, None));
    }
    if split.input_to_retirement > 0 {
        outputs.push(output(retirement, split.input_to_retirement, option, None));
    }
    if split.user_change > 0 {
        outputs.push(output(
            wallet,
            split.user_change,
            if direction == AmoebaDlmmSwapDirection::QuoteForOption {
                quote
            } else {
                option
            },
            params.user_input_has_delegate.then_some(user_delegate),
        ));
    }
    if split.output_from_compressed_pool > sponsor_from_compressed {
        outputs.push(output(
            wallet,
            split.output_from_compressed_pool - sponsor_from_compressed,
            if direction == AmoebaDlmmSwapDirection::QuoteForOption {
                option
            } else {
                quote
            },
            (direction == AmoebaDlmmSwapDirection::QuoteForOption).then_some(scope),
        ));
    }
    let sponsor_compressed_output = if buy {
        params.sponsor_fee_atoms
    } else {
        sponsor_from_compressed
    };
    if sponsor_compressed_output > 0 {
        let sponsor = wallet
            .checked_add(8)
            .ok_or(VaultError::InvalidAccountList)?;
        outputs.push(output(sponsor, sponsor_compressed_output, quote, None));
    }
    let mut metas = vec![
        AccountMeta::new_readonly(*context.light_system.key, false),
        AccountMeta::new(*payer.key, true),
        AccountMeta::new_readonly(*a[16].key, false),
        AccountMeta::new_readonly(*context.registered.key, false),
        AccountMeta::new_readonly(*context.compression_authority.key, false),
        AccountMeta::new_readonly(*context.compression_program.key, false),
        AccountMeta::new_readonly(*a[20].key, false),
    ];
    metas.extend(
        context
            .merkle
            .iter()
            .map(|info| AccountMeta::new(*info.key, false)),
    );
    metas.extend([
        AccountMeta::new_readonly(*trader.key, true),
        AccountMeta::new_readonly(*a[9].key, false),
        AccountMeta::new_readonly(*a[10].key, false),
        AccountMeta::new(*custody_info.key, true),
        AccountMeta::new(*context.writer_cash_custody.key, true),
        AccountMeta::new_readonly(*context.retirement.key, false),
        AccountMeta::new_readonly(*context.user_delegate.key, false),
        AccountMeta::new_readonly(*a[23].key, false),
        AccountMeta::new(*context.sponsor.key, true),
    ]);
    let ix = regular_compressed_transfer::instruction(
        *a[15].key,
        metas,
        params.output_queue_index,
        params.proof,
        &inputs,
        &outputs,
    )?;
    let mut infos = vec![
        context.light_system.clone(),
        payer.clone(),
        a[16].clone(),
        context.registered.clone(),
        context.compression_authority.clone(),
        context.compression_program.clone(),
        a[20].clone(),
    ];
    infos.extend(context.merkle.iter().cloned());
    infos.extend([
        trader.clone(),
        a[9].clone(),
        a[10].clone(),
        custody_info.clone(),
        context.writer_cash_custody.clone(),
        context.retirement.clone(),
        context.user_delegate.clone(),
        a[23].clone(),
        context.sponsor.clone(),
        a[15].clone(),
    ]);
    let pool_kind = [CustodyKind::Pool as u8];
    let cash_kind = [CustodyKind::WriterCash as u8];
    let pool_bump = [pool_before.bump];
    let cash_bump = [cash_before.bump];
    let pool_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::compressed_custody::COMPRESSED_CUSTODY_SEED,
        &pool_kind,
        pool_info.key.as_ref(),
        &pool_bump,
    ];
    let market = if resident.is_some() {
        Some(load_valid_market(program, custody_info)?)
    } else {
        None
    };
    let market_id = market.as_ref().map(|m| m.market_id).unwrap_or([0; 32]);
    let market_bump = market.as_ref().map(|m| [m.bump]).unwrap_or([0]);
    let market_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::constants::MARKET_PDA_SEED,
        &market_id,
        &market_bump,
    ];
    let pool_seeds = if resident.is_some() {
        market_seeds
    } else {
        pool_seeds
    };
    let cash_seeds: &[&[u8]] = &[
        CURRENT_STATE_NAMESPACE_SEED,
        crate::compressed_custody::COMPRESSED_CUSTODY_SEED,
        &cash_kind,
        a[27].key.as_ref(),
        &cash_bump,
    ];
    if let Some(owner) = context.trading_owner {
        let (expected, bump) = crate::trading_session::derive(program, &owner);
        if expected != *trader.key {
            return Err(VaultError::InvalidAccountList.into());
        }
        let bump = [bump];
        let trading_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            crate::trading_session::SEED,
            owner.as_ref(),
            &bump,
        ];
        invoke_signed(&ix, &infos, &[pool_seeds, cash_seeds, trading_seeds])?;
    } else {
        invoke_signed(&ix, &infos, &[pool_seeds, cash_seeds])?;
    }
    if resident.is_none() && context.pool_custody.owner == program {
        let mut pool_after = pool_before;
        pool_after.option_atoms = split.pool_option_after;
        pool_after.quote_atoms = split.pool_quote_after;
        crate::compressed_custody::store(context.pool_custody, &pool_after)?;
    }
    if resident.is_some() {
        context
            .after_pool
            .set(Some((split.pool_option_after, split.pool_quote_after)));
    }
    if writer_premium > 0 {
        let mut cash_after = cash_before;
        cash_after.quote_atoms = split.writer_quote_after;
        crate::compressed_custody::store(context.writer_cash_custody, &cash_after)?;
    }
    if split.output_from_hot > sponsor_from_hot {
        let compression = [
            context.light_system.clone(),
            context.registered.clone(),
            context.compression_authority.clone(),
            context.compression_program.clone(),
            context.merkle(params.output_queue_index)?.clone(),
        ];
        compressed_delivery::deliver(
            program,
            split.output_from_hot - sponsor_from_hot,
            output_vault,
            &a[8],
            authority_seeds,
            payer,
            trader,
            output_mint,
            output_interface,
            &a[15],
            &a[16],
            &a[19],
            &a[20],
            &compression,
            (direction == AmoebaDlmmSwapDirection::QuoteForOption).then_some(&a[23]),
        )?;
    }
    if sponsor_from_hot > 0 {
        let compression = [
            context.light_system.clone(),
            context.registered.clone(),
            context.compression_authority.clone(),
            context.compression_program.clone(),
            context.merkle(params.output_queue_index)?.clone(),
        ];
        compressed_delivery::deliver(
            program,
            sponsor_from_hot,
            output_vault,
            &a[8],
            authority_seeds,
            payer,
            context.sponsor,
            output_mint,
            output_interface,
            &a[15],
            &a[16],
            &a[19],
            &a[20],
            &compression,
            None,
        )?;
    }
    let _ = input_mint;
    Ok(split)
}
