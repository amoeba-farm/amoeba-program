use super::*;
use crate::{
    ameba_dlmm_instruction::{CompressedSwapLeafWitnessV1, RemoveCompressedLiquidityV1Params},
    compressed_custody::{
        derive_compressed_custody, CompressedCustodyV1, CustodyKind, COMPRESSED_CUSTODY_SEED,
    },
    compressed_swap_plan::plan_lp_withdrawal,
    constants::MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES,
    regular_compressed_transfer::{self, HotCompression, InputLeaf, OutputLeaf},
};
use solana_program::instruction::AccountMeta;

const FIXED: usize = 16;
const TAIL: usize = 5;

pub(super) struct Context<'a, 'info> {
    pub(super) before: CompressedCustodyV1,
    pub(super) sidecar: &'a AccountInfo<'info>,
    light_system: &'a AccountInfo<'info>,
    registered: &'a AccountInfo<'info>,
    compression_authority: &'a AccountInfo<'info>,
    compression_program: &'a AccountInfo<'info>,
    merkle: &'a [AccountInfo<'info>],
    params: RemoveCompressedLiquidityV1Params,
    resident: Option<(
        &'a AccountInfo<'info>,
        crate::market_router::ResidentRouterState,
    )>,
    after: std::cell::Cell<(u64, u64)>,
}

pub(super) fn process<'info>(
    program: &Pubkey,
    accounts: &[AccountInfo<'info>],
    params: RemoveCompressedLiquidityV1Params,
) -> ProgramResult {
    let supplied = accounts;
    let pool_info = accounts.get(1).ok_or(VaultError::InvalidAccountList)?;
    let accounts = without_market_tail(program, pool_info, accounts)?;
    if params.entries.is_empty()
        || params.entries.len() > MAX_AMOEBA_DLMM_LIQUIDITY_ENTRIES
        || params
            .entries
            .windows(2)
            .any(|pair| pair[0].bin_id >= pair[1].bin_id)
        || params.page_count == 0
        || usize::from(params.page_count) > usize::from(MAX_AMOEBA_DLMM_PAGE_COUNT)
        || params.merkle_account_count < 2
        || usize::from(params.output_tree_index) >= usize::from(params.merkle_account_count)
        || usize::from(params.output_queue_index) >= usize::from(params.merkle_account_count)
        || params.output_tree_index == params.output_queue_index
    {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let page_end = FIXED + usize::from(params.page_count) * 2;
    if accounts.len() != page_end + TAIL + usize::from(params.merkle_account_count)
        || accounts.len() > 255
    {
        return Err(VaultError::InvalidAccountList.into());
    }
    let a = &accounts[..page_end];
    let tail = &accounts[page_end..];
    let merkle = &tail[TAIL..];
    let pool = load_pool_with_accounts(program, &a[1], supplied)?;
    let resident = resident_for_pool(program, &a[1], supplied)?;
    if !a[0].is_signer
        || !a[0].is_writable
        || !a[1].is_writable
        || !a[2].is_writable
        || *a[0].key != pool.liquidity_manager
        || a[8].key != a[0].key
        || a[9].key != a[0].key
        || *tail[0].key != derive_compressed_custody(program, CustodyKind::Pool, a[1].key).0
        || !tail[0].is_writable
        || tail[0].is_signer
        || !crate::light_token_instruction::is_light_system_program(tail[1].key)
        || !crate::light_token_instruction::is_registered_program(tail[2].key)
        || !crate::light_token_instruction::is_compression_authority(tail[3].key)
        || !crate::light_token_instruction::is_compression_program(tail[4].key)
        || tail[1..TAIL]
            .iter()
            .any(|info| info.is_signer || info.is_writable)
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
    for witness in [params.pool_option_input, params.pool_quote_input]
        .into_iter()
        .flatten()
    {
        if usize::from(witness.tree_index) >= merkle.len()
            || usize::from(witness.queue_index) >= merkle.len()
            || witness.tree_index == witness.queue_index
            || (params.proof.is_none() && !witness.prove_by_index)
        {
            return Err(VaultError::InvalidInstructionData.into());
        }
    }
    let before = if let Some((_, state)) = &resident {
        pool_ledger(
            program,
            a[1].key,
            &pool,
            state.pool_option,
            state.pool_quote,
        )
    } else {
        load_or_create_custody(
            program,
            &a[0],
            &tail[0],
            &a[15],
            CustodyKind::Pool,
            a[1].key,
            &pool.option_mint,
            &pool.quote_mint,
        )?
    };
    let context = Context {
        after: std::cell::Cell::new((before.option_atoms, before.quote_atoms)),
        before,
        sidecar: &tail[0],
        light_system: &tail[1],
        registered: &tail[2],
        compression_authority: &tail[3],
        compression_program: &tail[4],
        merkle,
        params: params.clone(),
        resident,
    };
    let normalized = with_resident_market(program, &a[1], supplied, a.to_vec())?;
    process_liquidity_change_core(
        program,
        &normalized,
        LiquidityChange::Remove(RemoveAmoebaDlmmLiquidityV1Params {
            position_nonce: params.position_nonce,
            entries: params.entries,
            close_position_when_empty: params.close_position_when_empty,
        }),
        None,
        Some(&context),
    )
}

fn input(witness: CompressedSwapLeafWitnessV1, owner: u8, mint: u8, amount: u64) -> InputLeaf {
    InputLeaf {
        owner,
        amount,
        has_delegate: false,
        delegate: 0,
        mint,
        tree: witness.tree_index,
        queue: witness.queue_index,
        leaf_index: witness.leaf_index,
        prove_by_index: witness.prove_by_index,
        root_index: witness.root_index,
    }
}

impl<'info> Context<'_, 'info> {
    pub(super) fn after_cold(&self) -> (u64, u64) {
        self.after.get()
    }
    pub(super) fn settle(
        &self,
        program: &Pubkey,
        a: &[AccountInfo<'info>],
        pool: &AmoebaDlmmPoolV1,
        vaults: &(TokenAccount, TokenAccount),
        option_out: u64,
        quote_out: u64,
    ) -> ProgramResult {
        let old_option = pool
            .accounted_option_reserve
            .checked_add(option_out)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let old_quote = pool
            .accounted_quote_reserve
            .checked_add(quote_out)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let option = plan_lp_withdrawal(
            vaults.0.amount,
            self.before.option_atoms,
            old_option,
            option_out,
        )?;
        let quote = plan_lp_withdrawal(
            vaults.1.amount,
            self.before.quote_atoms,
            old_quote,
            quote_out,
        )?;
        if self.params.pool_option_input.is_some() != (option.compressed_input > 0)
            || self.params.pool_quote_input.is_some() != (quote.compressed_input > 0)
        {
            return Err(VaultError::InvalidAccountList.into());
        }
        if option_out == 0 && quote_out == 0 {
            return Ok(());
        }
        let count = u8::try_from(self.merkle.len()).map_err(|_| VaultError::InvalidAccountList)?;
        let wallet = count;
        let option_mint = count.checked_add(1).ok_or(VaultError::InvalidAccountList)?;
        let quote_mint = count.checked_add(2).ok_or(VaultError::InvalidAccountList)?;
        let custody = count.checked_add(3).ok_or(VaultError::InvalidAccountList)?;
        let option_vault = count.checked_add(4).ok_or(VaultError::InvalidAccountList)?;
        let quote_vault = count.checked_add(5).ok_or(VaultError::InvalidAccountList)?;
        let authority = count.checked_add(6).ok_or(VaultError::InvalidAccountList)?;
        let option_interface = count.checked_add(7).ok_or(VaultError::InvalidAccountList)?;
        let quote_interface = count.checked_add(8).ok_or(VaultError::InvalidAccountList)?;
        let mut inputs = Vec::with_capacity(2);
        let physical_option = self
            .resident
            .as_ref()
            .map_or(Ok(option.compressed_input), |(_, s)| s.total_option())?;
        let physical_quote = self
            .resident
            .as_ref()
            .map_or(Ok(quote.compressed_input), |(_, s)| s.total_quote())?;
        if let Some(w) = self.params.pool_option_input {
            inputs.push(input(w, custody, option_mint, physical_option));
        }
        if let Some(w) = self.params.pool_quote_input {
            inputs.push(input(w, custody, quote_mint, physical_quote));
        }
        let mut compressions = Vec::with_capacity(2);
        for (amount, mint, source, interface, key) in [
            (
                option.hot_compression,
                option_mint,
                option_vault,
                option_interface,
                a[4].key,
            ),
            (
                quote.hot_compression,
                quote_mint,
                quote_vault,
                quote_interface,
                a[5].key,
            ),
        ] {
            if amount > 0 {
                compressions.push(HotCompression {
                    amount,
                    mint,
                    source,
                    authority,
                    pool_account_index: interface,
                    pool_index: 0,
                    bump: crate::light_token_instruction::get_spl_interface_pda_and_bump(key).1,
                    decimals: MarketMintAccounting::CANONICAL_DECIMALS,
                });
            }
        }
        let mut outputs = Vec::with_capacity(4);
        for (amount, mint) in [
            (option.wallet_output, option_mint),
            (quote.wallet_output, quote_mint),
        ] {
            if amount > 0 {
                outputs.push(OutputLeaf {
                    owner: wallet,
                    amount,
                    has_delegate: false,
                    delegate: 0,
                    mint,
                });
            }
        }
        for (spent, change, mint) in [
            (
                option.compressed_input,
                option
                    .compressed_change
                    .checked_add(self.resident.as_ref().map_or(0, |(_, s)| s.book_option))
                    .ok_or(VaultError::ArithmeticOverflow)?,
                option_mint,
            ),
            (
                quote.compressed_input,
                quote
                    .compressed_change
                    .checked_add(self.resident.as_ref().map_or(0, |(_, s)| s.book_quote))
                    .ok_or(VaultError::ArithmeticOverflow)?,
                quote_mint,
            ),
        ] {
            if spent > 0 && change > 0 {
                outputs.push(OutputLeaf {
                    owner: custody,
                    amount: change,
                    has_delegate: false,
                    delegate: 0,
                    mint,
                });
            }
        }
        let mut metas = vec![
            AccountMeta::new_readonly(*self.light_system.key, false),
            AccountMeta::new(*a[0].key, true),
            AccountMeta::new_readonly(*a[11].key, false),
            AccountMeta::new_readonly(*self.registered.key, false),
            AccountMeta::new_readonly(*self.compression_authority.key, false),
            AccountMeta::new_readonly(*self.compression_program.key, false),
            AccountMeta::new_readonly(*a[15].key, false),
        ];
        metas.extend(
            self.merkle
                .iter()
                .map(|info| AccountMeta::new(*info.key, false)),
        );
        metas.extend([
            AccountMeta::new_readonly(*a[0].key, true),
            AccountMeta::new_readonly(*a[4].key, false),
            AccountMeta::new_readonly(*a[5].key, false),
            AccountMeta::new(
                *self
                    .resident
                    .as_ref()
                    .map_or(self.sidecar, |(market, _)| *market)
                    .key,
                true,
            ),
            AccountMeta::new(*a[6].key, false),
            AccountMeta::new(*a[7].key, false),
            AccountMeta::new_readonly(*a[3].key, true),
            AccountMeta::new(*a[12].key, false),
            AccountMeta::new(*a[13].key, false),
            AccountMeta::new_readonly(*a[14].key, false),
        ]);
        let ix = regular_compressed_transfer::instruction_with_compressions(
            *a[10].key,
            metas,
            self.params.output_queue_index,
            self.params.proof,
            &inputs,
            &compressions,
            &outputs,
        )?;
        let mut infos = vec![
            self.light_system.clone(),
            a[0].clone(),
            a[11].clone(),
            self.registered.clone(),
            self.compression_authority.clone(),
            self.compression_program.clone(),
            a[15].clone(),
        ];
        infos.extend(self.merkle.iter().cloned());
        infos.extend([
            a[0].clone(),
            a[4].clone(),
            a[5].clone(),
            self.resident
                .as_ref()
                .map_or(self.sidecar, |(market, _)| *market)
                .clone(),
            a[6].clone(),
            a[7].clone(),
            a[3].clone(),
            a[12].clone(),
            a[13].clone(),
            a[14].clone(),
            a[10].clone(),
        ]);
        let kind = [CustodyKind::Pool as u8];
        let custody_bump = [self.before.bump];
        let custody_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            COMPRESSED_CUSTODY_SEED,
            &kind,
            a[1].key.as_ref(),
            &custody_bump,
        ];
        let (_, authority_bump) = derive_ameba_dlmm_authority_pda(program, a[1].key);
        let authority_bump = [authority_bump];
        let authority_seeds: &[&[u8]] = &[
            CURRENT_STATE_NAMESPACE_SEED,
            AMOEBA_DLMM_AUTHORITY_PDA_SEED,
            a[1].key.as_ref(),
            &authority_bump,
        ];
        if let Some((market, _)) = &self.resident {
            let state = load_valid_market(program, market)?;
            let bump = [state.bump];
            let seeds: &[&[u8]] = &[
                CURRENT_STATE_NAMESPACE_SEED,
                crate::constants::MARKET_PDA_SEED,
                &state.market_id,
                &bump,
            ];
            invoke_signed(&ix, &infos, &[seeds, authority_seeds])?;
        } else {
            invoke_signed(&ix, &infos, &[custody_seeds, authority_seeds])?;
        }
        let mut after = self.before.clone();
        after.option_atoms = option.compressed_change;
        after.quote_atoms = quote.compressed_change;
        if self.resident.is_none() {
            crate::compressed_custody::store(self.sidecar, &after)?;
        }
        self.after.set((after.option_atoms, after.quote_atoms));
        Ok(())
    }
}
