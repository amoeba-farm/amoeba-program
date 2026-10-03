use super::*;

/// The five compression accounts are appended after route pages and recipient
/// wallets. They never participate in the page or order-witness parsers.
pub(in crate::processor) const COMPRESSION_ACCOUNTS: usize = 5;

#[allow(clippy::too_many_arguments)]
pub(in crate::processor) fn deliver<'a>(
    program: &Pubkey,
    amount: u64,
    source: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    authority_seeds: &[&[u8]],
    payer: &AccountInfo<'a>,
    recipient: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    interface: &AccountInfo<'a>,
    light: &AccountInfo<'a>,
    cpi_authority: &AccountInfo<'a>,
    spl: &AccountInfo<'a>,
    system: &AccountInfo<'a>,
    compression: &[AccountInfo<'a>],
    delegate_info: Option<&AccountInfo<'a>>,
) -> ProgramResult {
    if compression.len() != COMPRESSION_ACCOUNTS || recipient.executable || amount == 0 {
        return Err(VaultError::InvalidAccountList.into());
    }
    if let Some(info) = delegate_info {
        if *info.key
            != crate::scoped_settlement::derive_collective_settlement_delegate(
                program,
                recipient.key,
                mint.key,
            )
            .0
            || info.executable
            || info.is_writable
            || info.is_signer
        {
            return Err(VaultError::InvalidAccountList.into());
        }
    }
    let ix = light_token_instruction::compress_to_wallet_with_delegate(
        amount,
        MarketMintAccounting::CANONICAL_DECIMALS,
        source.key,
        source.owner,
        authority.key,
        payer.key,
        mint.key,
        recipient.key,
        interface.key,
        [
            compression[0].key,
            compression[1].key,
            compression[2].key,
            compression[3].key,
            compression[4].key,
        ],
        delegate_info.map(|info| info.key),
    )?;
    let mut infos = vec![
        compression[0].clone(),
        payer.clone(),
        cpi_authority.clone(),
        compression[1].clone(),
        compression[2].clone(),
        compression[3].clone(),
        system.clone(),
        compression[4].clone(),
        mint.clone(),
        source.clone(),
        authority.clone(),
        recipient.clone(),
        interface.clone(),
        spl.clone(),
        light.clone(),
    ];
    if let Some(info) = delegate_info {
        infos.push(info.clone());
    }
    invoke_signed(&ix, &infos, &[authority_seeds])?;
    Ok(())
}
