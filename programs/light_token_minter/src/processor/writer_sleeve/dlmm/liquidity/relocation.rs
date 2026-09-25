//! Atomic redistribution within one canonical writer position. No token CPI or liability mutation.
use super::*;
use crate::state::{WriterDlmmPositionV1, WRITER_DLMM_ACTION_ENTRIES};
use crate::writer_dlmm_instruction::WriterDlmmMoveV1;

fn aggregate(
    entries: &mut Vec<WriterDlmmBinV1>,
    id: u16,
    options: u64,
    quote: u64,
) -> ProgramResult {
    match entries.binary_search_by_key(&id, |entry| entry.bin_id) {
        Ok(index) => {
            entries[index].option_atoms = entries[index]
                .option_atoms
                .checked_add(options)
                .ok_or(VaultError::ArithmeticOverflow)?;
            entries[index].quote_atoms = entries[index]
                .quote_atoms
                .checked_add(quote)
                .ok_or(VaultError::ArithmeticOverflow)?;
        }
        Err(index) => entries.insert(
            index,
            WriterDlmmBinV1 {
                bin_id: id,
                option_atoms: options,
                quote_atoms: quote,
            },
        ),
    }
    Ok(())
}

pub(super) fn relocate(
    position: &mut WriterDlmmPositionV1,
    moves: &[WriterDlmmMoveV1],
    maximum_bin: u16,
    tick: u64,
    minimum_ask: u64,
    maximum_bid: u64,
) -> ProgramResult {
    if moves.is_empty() || moves.len() > WRITER_DLMM_ACTION_ENTRIES {
        return Err(VaultError::InvalidInstructionData.into());
    }
    let before = (
        position.option_inventory_atoms,
        position.allocated_quote_atoms,
        position.uncommitted_quote_atoms,
    );
    let mut removals = Vec::with_capacity(moves.len());
    let mut additions = Vec::with_capacity(moves.len());
    let mut previous = None;
    for movement in moves {
        let pair = (movement.source_bin_id, movement.destination_bin_id);
        if movement.source_bin_id == 0
            || movement.source_bin_id > maximum_bin
            || movement.destination_bin_id == 0
            || movement.destination_bin_id > maximum_bin
            || movement.source_bin_id == movement.destination_bin_id
            || (movement.option_atoms == 0 && movement.quote_atoms == 0)
            || previous.is_some_and(|value| value >= pair)
        {
            return Err(VaultError::InvalidInstructionData.into());
        }
        previous = Some(pair);
        let price = u64::from(movement.destination_bin_id)
            .checked_mul(tick)
            .ok_or(VaultError::ArithmeticOverflow)?;
        if (movement.option_atoms > 0 && price < minimum_ask)
            || (movement.quote_atoms > 0 && price > maximum_bid)
        {
            return Err(VaultError::InvalidWriterPolicySnapshot.into());
        }
        aggregate(
            &mut removals,
            movement.source_bin_id,
            movement.option_atoms,
            movement.quote_atoms,
        )?;
        aggregate(
            &mut additions,
            movement.destination_bin_id,
            movement.option_atoms,
            movement.quote_atoms,
        )?;
    }
    // Debit all sources before crediting any destination: cycles cannot borrow new credits.
    apply_entries(position, &removals, maximum_bin, false)?;
    apply_entries(position, &additions, maximum_bin, true)?;
    if before
        != (
            position.option_inventory_atoms,
            position.allocated_quote_atoms,
            position.uncommitted_quote_atoms,
        )
    {
        return Err(VaultError::WriterSupplyMismatch.into());
    }
    Ok(())
}
