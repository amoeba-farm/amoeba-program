use super::*;
use crate::{ameba_dlmm_math::AMOEBA_DLMM_BINS_PER_PAGE, constants::MAX_AMOEBA_DLMM_PAGE_COUNT};

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub(super) struct CompressAndCloseParams {
    pub(super) proof: ValidityProof,
    pub(super) compressed_accounts: Vec<CompressedAccountMetaNoLamportsNoAddress>,
    pub(super) system_accounts_offset: u8,
}

#[derive(Clone, BorshSerialize)]
pub(super) struct PackedCompressedAccountData {
    pub(super) tree_info: PackedStateTreeInfo,
    pub(super) data: DecodedAmoebaDlmmState,
}

#[derive(Clone, BorshSerialize)]
pub(super) struct DecompressIdempotentParams {
    pub(super) system_accounts_offset: u8,
    pub(super) token_accounts_offset: u8,
    pub(super) output_queue_index: u8,
    pub(super) proof: ValidityProof,
    pub(super) accounts: Vec<PackedCompressedAccountData>,
}

#[inline(never)]
pub(super) fn read_packed_variant(
    reader: &mut CheckedCursor<'_>,
) -> Result<DecodedAmoebaDlmmState, ProgramError> {
    let kind = AmoebaDlmmStateKind::from_tag(reader.u8())?;
    let body = reader.slice(kind.body_len());
    let state = DecodedAmoebaDlmmState::from_body(kind, body)?;
    match kind {
        AmoebaDlmmStateKind::Pool => {}
        AmoebaDlmmStateKind::BinPage | AmoebaDlmmStateKind::SharePage => {
            if reader.bytes::<2>() != state.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2] {
                return Err(invalid_light());
            }
        }
        AmoebaDlmmStateKind::Position => {
            if reader.bytes::<8>() != state.body[POSITION_NONCE_OFFSET..POSITION_NONCE_OFFSET + 8] {
                return Err(invalid_light());
            }
        }
    }
    Ok(state)
}

#[inline(never)]
pub(super) fn decode_compress_params(
    payload: &[u8],
) -> Result<CompressAndCloseParams, ProgramError> {
    let mut reader = CheckedCursor::new(payload);
    let proof = deserialize_validity_proof_cursor(&mut reader);
    let count = reader.u32() as usize;
    if count
        .checked_mul(10)
        .and_then(|length| length.checked_add(1))
        != Some(reader.remaining_len())
    {
        return Err(invalid_light());
    }
    let mut compressed_accounts = Vec::with_capacity(count);
    for _ in 0..count {
        compressed_accounts.push(CompressedAccountMetaNoLamportsNoAddress {
            tree_info: deserialize_packed_state_tree_info_cursor(&mut reader),
            output_state_tree_index: reader.u8(),
        });
    }
    let system_accounts_offset = reader.u8();
    reader.finish_exact().map_err(|_| invalid_light())?;
    Ok(CompressAndCloseParams {
        proof: proof.into(),
        compressed_accounts,
        system_accounts_offset,
    })
}

#[inline(never)]
pub(super) fn decode_decompress_params(
    payload: &[u8],
) -> Result<DecompressIdempotentParams, ProgramError> {
    let mut reader = CheckedCursor::new(payload);
    let system_accounts_offset = reader.u8();
    let token_accounts_offset = reader.u8();
    let output_queue_index = reader.u8();
    let proof = deserialize_validity_proof_cursor(&mut reader);
    let count = reader.u32() as usize;
    if count > reader.remaining_len() / 10 {
        return Err(invalid_light());
    }
    let mut accounts = Vec::with_capacity(count);
    for _ in 0..count {
        accounts.push(PackedCompressedAccountData {
            tree_info: deserialize_packed_state_tree_info_cursor(&mut reader),
            data: read_packed_variant(&mut reader)?,
        });
    }
    reader.finish_exact().map_err(|_| invalid_light())?;
    Ok(DecompressIdempotentParams {
        system_accounts_offset,
        token_accounts_offset,
        output_queue_index,
        proof: proof.into(),
        accounts,
    })
}

pub(super) fn load_config_and_sponsor(
    program_id: &Pubkey,
    config_info: &AccountInfo,
    rent_sponsor: &AccountInfo,
) -> Result<(AmoebaLightConfig, u8), ProgramError> {
    let config = load_light_config(program_id, config_info)?;
    if config.rent_sponsor != rent_sponsor.key.to_bytes()
        || !rent_sponsor.is_writable
        || rent_sponsor.executable
        || rent_sponsor.owner != &system_program::id()
    {
        return Err(invalid_light());
    }
    let bump = [config.rent_sponsor_bump];
    let expected = Pubkey::create_program_address(&[RENT_SPONSOR_SEED, &bump], program_id)
        .map_err(|_| invalid_light())?;
    if expected != *rent_sponsor.key {
        return Err(invalid_light());
    }
    Ok((config, bump[0]))
}

pub(super) const COMPRESSION_INFO_LEN: usize = 24;
pub(super) const POOL_STATUS_OFFSET: usize = 327;
pub(super) const POOL_OR_PAGE_IDENTITY_OFFSET: usize = 6;
pub(super) const PAGE_INDEX_OFFSET: usize = 38;
pub(super) const POSITION_OWNER_OFFSET: usize = 38;
pub(super) const POSITION_NONCE_OFFSET: usize = 70;
const PAGE_FIRST_BIN_OFFSET: usize = 40;
const PAGE_EMPTY_START: usize = 42;
const BIN_PAGE_EMPTY_END: usize = 562;
const SHARE_PAGE_EMPTY_END: usize = 554;
const POSITION_LOWER_BIN_OFFSET: usize = 78;
const POSITION_BIN_COUNT_OFFSET: usize = 80;
const POSITION_EMPTY_START: usize = 81;
const POSITION_EMPTY_END: usize = 597;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum AmoebaDlmmStateKind {
    Pool = 0,
    BinPage = 1,
    SharePage = 2,
    Position = 3,
}

impl AmoebaDlmmStateKind {
    fn from_tag(tag: u8) -> Result<Self, ProgramError> {
        match tag {
            0 => Ok(Self::Pool),
            1 => Ok(Self::BinPage),
            2 => Ok(Self::SharePage),
            3 => Ok(Self::Position),
            _ => Err(invalid_light()),
        }
    }

    fn from_discriminator(discriminator: &[u8]) -> Result<Self, ProgramError> {
        match discriminator {
            value if value == AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR => Ok(Self::Pool),
            value if value == AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR => Ok(Self::BinPage),
            value if value == AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR => Ok(Self::SharePage),
            value if value == AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR => Ok(Self::Position),
            _ => Err(invalid_light()),
        }
    }

    fn body_len(self) -> usize {
        match self {
            Self::Pool => AmoebaDlmmPoolV1::BODY_LEN,
            Self::BinPage => AmoebaDlmmBinPageV1::BODY_LEN,
            Self::SharePage => AmoebaDlmmSharePageV1::BODY_LEN,
            Self::Position => AmoebaDlmmPositionV1::BODY_LEN,
        }
    }

    fn account_len(self) -> usize {
        self.body_len() + 8
    }

    fn discriminator(self) -> [u8; 8] {
        match self {
            Self::Pool => AMOEBA_DLMM_POOL_LIGHT_DISCRIMINATOR,
            Self::BinPage => AMOEBA_DLMM_BIN_PAGE_LIGHT_DISCRIMINATOR,
            Self::SharePage => AMOEBA_DLMM_SHARE_PAGE_LIGHT_DISCRIMINATOR,
            Self::Position => AMOEBA_DLMM_POSITION_LIGHT_DISCRIMINATOR,
        }
    }

    fn account_discriminator(self) -> [u8; 3] {
        match self {
            Self::Pool => crate::ameba_dlmm_state::AMOEBA_DLMM_POOL_ACCOUNT_DISCRIMINATOR,
            Self::BinPage => crate::ameba_dlmm_state::AMOEBA_DLMM_BIN_PAGE_ACCOUNT_DISCRIMINATOR,
            Self::SharePage => {
                crate::ameba_dlmm_state::AMOEBA_DLMM_SHARE_PAGE_ACCOUNT_DISCRIMINATOR
            }
            Self::Position => crate::ameba_dlmm_state::AMOEBA_DLMM_POSITION_ACCOUNT_DISCRIMINATOR,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct DecodedAmoebaDlmmState {
    pub(super) kind: AmoebaDlmmStateKind,
    pub(super) body: Vec<u8>,
}

impl DecodedAmoebaDlmmState {
    pub(super) fn from_body(kind: AmoebaDlmmStateKind, body: &[u8]) -> Result<Self, ProgramError> {
        if body.len() != kind.body_len()
            || body[0] > 1
            || (kind == AmoebaDlmmStateKind::Pool && body[POOL_STATUS_OFFSET] > 4)
            || body[body.len() - 10] > CompressionState::Compressed as u8
        {
            return Err(invalid_light());
        }
        Ok(Self {
            kind,
            body: body.to_vec(),
        })
    }

    fn decode(program_id: &Pubkey, account: &AccountInfo) -> Result<Self, ProgramError> {
        if account.owner != program_id || account.executable || !account.is_writable {
            return Err(invalid_light());
        }
        let data = account.try_borrow_data().map_err(|_| invalid_light())?;
        if data.len() < 8 {
            return Err(invalid_light());
        }
        let kind = AmoebaDlmmStateKind::from_discriminator(&data[..8])?;
        Self::from_body(kind, &data[8..])
    }

    fn discriminator(&self) -> [u8; 8] {
        self.kind.discriminator()
    }

    pub(super) fn compression_info(&self) -> Result<CompressionInfo, ProgramError> {
        let start = self.body.len() - COMPRESSION_INFO_LEN;
        let mut input = FixedCursor::new(&self.body[start..]);
        let info = <CompressionInfo as FixedField>::read(&mut input);
        if input.invalid || input.offset != COMPRESSION_INFO_LEN {
            return Err(invalid_light());
        }
        Ok(info)
    }

    fn write_compression_info(body: &mut [u8], info: &CompressionInfo) {
        let start = body.len() - COMPRESSION_INFO_LEN;
        let mut output = FixedWriter::new(&mut body[start..]);
        <CompressionInfo as FixedField>::write(info, &mut output);
        debug_assert_eq!(output.offset, COMPRESSION_INFO_LEN);
    }

    fn has_header_with_state(&self, state: CompressionState) -> bool {
        self.body[2..5] == self.kind.account_discriminator()
            && self.body[5] == crate::ameba_dlmm_state::AMOEBA_DLMM_ACCOUNT_VERSION
            && self.body[self.body.len() - 10] == state as u8
    }

    pub(super) fn has_layout_with_state(&self, state: CompressionState) -> bool {
        self.body[0] == 1 && self.has_header_with_state(state)
    }

    /// Terminal positions and page pairs remain registered Light addresses, so
    /// they must pass through the compression CPI before their hot PDAs close.
    /// Keep this separate from active loaders: only the exact empty states
    /// written by the terminalization instructions are admitted here.
    pub(super) fn has_terminal_tombstone_layout(&self) -> bool {
        if self.body[0] != 0 || !self.has_header_with_state(CompressionState::Decompressed) {
            return false;
        }
        let identity_is_nonzero = self.body
            [POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32]
            .iter()
            .any(|byte| *byte != 0);
        match self.kind {
            AmoebaDlmmStateKind::Pool => false,
            AmoebaDlmmStateKind::BinPage | AmoebaDlmmStateKind::SharePage => {
                let page_index = u16::from_le_bytes([
                    self.body[PAGE_INDEX_OFFSET],
                    self.body[PAGE_INDEX_OFFSET + 1],
                ]);
                let first_bin_id = u16::from_le_bytes([
                    self.body[PAGE_FIRST_BIN_OFFSET],
                    self.body[PAGE_FIRST_BIN_OFFSET + 1],
                ]);
                let empty_end = if self.kind == AmoebaDlmmStateKind::BinPage {
                    BIN_PAGE_EMPTY_END
                } else {
                    SHARE_PAGE_EMPTY_END
                };
                identity_is_nonzero
                    && page_index < MAX_AMOEBA_DLMM_PAGE_COUNT
                    && first_bin_id == page_index * AMOEBA_DLMM_BINS_PER_PAGE + 1
                    && self.body[PAGE_EMPTY_START..empty_end]
                        .iter()
                        .all(|byte| *byte == 0)
            }
            AmoebaDlmmStateKind::Position => {
                let lower_bin_id = u16::from_le_bytes([
                    self.body[POSITION_LOWER_BIN_OFFSET],
                    self.body[POSITION_LOWER_BIN_OFFSET + 1],
                ]);
                let bin_count = self.body[POSITION_BIN_COUNT_OFFSET];
                identity_is_nonzero
                    && self.body[POSITION_OWNER_OFFSET..POSITION_OWNER_OFFSET + 32]
                        .iter()
                        .any(|byte| *byte != 0)
                    && (1..=32).contains(&bin_count)
                    && lower_bin_id != 0
                    && lower_bin_id.checked_add(bin_count as u16 - 1).is_some()
                    && self.body[POSITION_EMPTY_START..POSITION_EMPTY_END]
                        .iter()
                        .all(|byte| *byte == 0)
            }
        }
    }

    pub(super) fn validate_pda(&self, program_id: &Pubkey, key: &Pubkey) -> ProgramResult {
        let bump = [self.body[1]];
        let expected = match self.kind {
            AmoebaDlmmStateKind::Pool => Pubkey::create_program_address(
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_POOL_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &bump,
                ],
                program_id,
            ),
            AmoebaDlmmStateKind::BinPage => Pubkey::create_program_address(
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_BIN_PAGE_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2],
                    &bump,
                ],
                program_id,
            ),
            AmoebaDlmmStateKind::SharePage => Pubkey::create_program_address(
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_SHARE_PAGE_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2],
                    &bump,
                ],
                program_id,
            ),
            AmoebaDlmmStateKind::Position => Pubkey::create_program_address(
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_POSITION_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[POSITION_OWNER_OFFSET..POSITION_OWNER_OFFSET + 32],
                    &self.body[POSITION_NONCE_OFFSET..POSITION_NONCE_OFFSET + 8],
                    &bump,
                ],
                program_id,
            ),
        }
        .map_err(|_| invalid_light())?;
        if expected != *key {
            return Err(invalid_light());
        }
        Ok(())
    }

    pub(super) fn compressed_body(&self) -> Vec<u8> {
        let mut body = self.body.clone();
        Self::write_compression_info(&mut body, &CompressionInfo::compressed());
        body
    }

    fn same_identity(&self, other: &Self) -> bool {
        if self.kind != other.kind || self.body[1] != other.body[1] {
            return false;
        }
        match self.kind {
            AmoebaDlmmStateKind::Pool => self.body[6..38] == other.body[6..38],
            AmoebaDlmmStateKind::BinPage | AmoebaDlmmStateKind::SharePage => {
                self.body[6..40] == other.body[6..40]
            }
            AmoebaDlmmStateKind::Position => self.body[6..78] == other.body[6..78],
        }
    }
}

impl BorshSerialize for DecodedAmoebaDlmmState {
    fn serialize<W: Write>(&self, writer: &mut W) -> IoResult<()> {
        (self.kind as u8).serialize(writer)?;
        writer.write_all(&self.body)?;
        match self.kind {
            AmoebaDlmmStateKind::Pool => Ok(()),
            AmoebaDlmmStateKind::BinPage | AmoebaDlmmStateKind::SharePage => {
                writer.write_all(&self.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2])
            }
            AmoebaDlmmStateKind::Position => {
                writer.write_all(&self.body[POSITION_NONCE_OFFSET..POSITION_NONCE_OFFSET + 8])
            }
        }
    }
}

pub(super) fn is_compressible(
    account: &AccountInfo,
    state: &DecodedAmoebaDlmmState,
    current_slot: u64,
    rent: &Rent,
) -> Result<bool, ProgramError> {
    let info = state.compression_info()?;
    Ok(AccountRentState {
        num_bytes: account.data_len() as u64,
        current_slot,
        current_lamports: account.lamports(),
        last_claimed_slot: info.last_claimed_slot,
    }
    .is_compressible(&info.rent_config, rent.minimum_balance(account.data_len()))
    .is_some())
}

#[inline(never)]
pub(super) fn execute_compress(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: &CompressAndCloseParams,
) -> ProgramResult {
    let num_pdas = params.compressed_accounts.len();
    if num_pdas == 0 || accounts.len() < 3 {
        return Err(invalid_light());
    }
    let system_accounts_offset = params.system_accounts_offset as usize;
    let pda_start = accounts
        .len()
        .checked_sub(num_pdas)
        .ok_or_else(invalid_light)?;
    if system_accounts_offset > pda_start
        || pda_start < 3
        || pda_start.saturating_sub(system_accounts_offset) < LIGHT_FIXED_ACCOUNTS
        || !accounts[0].is_signer
        || !accounts[0].is_writable
    {
        return Err(invalid_light());
    }
    let (config, _) = load_config_and_sponsor(program_id, &accounts[1], &accounts[2])?;
    let address_tree = config.address_tree;
    let current_slot = Clock::get().map_err(|_| invalid_light())?.slot;
    let rent = Rent::get().map_err(|_| invalid_light())?;

    let mut account_infos = Vec::with_capacity(num_pdas);
    for (index, meta) in params.compressed_accounts.iter().enumerate() {
        let pda_index = pda_start + index;
        let account = &accounts[pda_index];
        let state = DecodedAmoebaDlmmState::decode(program_id, account)?;
        if !state.has_layout_with_state(CompressionState::Decompressed)
            && !state.has_terminal_tombstone_layout()
        {
            return Err(invalid_light());
        }
        state.validate_pda(program_id, account.key)?;
        if !is_compressible(account, &state, current_slot, &rent)? {
            return Ok(());
        }
        let body = state.compressed_body();
        let address = derive_assigned_address(
            &account.key.to_bytes(),
            &address_tree,
            &program_id.to_bytes(),
        );
        account_infos.push(CompressedAccountInfo {
            address: Some(address),
            input: Some(InAccountInfo {
                discriminator: DECOMPRESSED_PDA_DISCRIMINATOR,
                data_hash: hash_leaf_data(account.key.as_ref()).map_err(|_| invalid_light())?,
                merkle_context: PackedMerkleContext {
                    merkle_tree_pubkey_index: meta.tree_info.merkle_tree_pubkey_index,
                    queue_pubkey_index: meta.tree_info.queue_pubkey_index,
                    leaf_index: meta.tree_info.leaf_index,
                    prove_by_index: meta.tree_info.prove_by_index,
                },
                root_index: if meta.tree_info.prove_by_index {
                    0
                } else {
                    meta.tree_info.root_index
                },
                lamports: 0,
            }),
            output: Some(OutAccountInfo {
                discriminator: state.discriminator(),
                data_hash: hash_leaf_data(&body).map_err(|_| invalid_light())?,
                output_merkle_tree_index: meta.output_state_tree_index,
                lamports: 0,
                data: body,
            }),
        });
    }

    let mut instruction = new_light_system_cpi(params.proof);
    instruction.account_infos = account_infos;
    invoke_light_cpi(
        instruction,
        &accounts[0],
        &accounts[system_accounts_offset..pda_start],
    )
    .map_err(|_| invalid_light())?;

    for account in &accounts[pda_start..] {
        super::super::close_program_account(program_id, account, &accounts[2])
            .map_err(|_| invalid_light())?;
    }
    Ok(())
}

#[inline(never)]
pub(super) fn process_compress(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params = decode_compress_params(payload)?;
    execute_compress(program_id, accounts, &params)
}

pub(super) fn create_hot_pda<'info>(
    program_id: &Pubkey,
    target: &AccountInfo<'info>,
    rent_sponsor: &AccountInfo<'info>,
    system_program_info: &AccountInfo<'info>,
    rent_sponsor_bump: u8,
    account_len: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    if !target.is_writable
        || target.executable
        || target.owner != &system_program::id()
        || target.data_len() != 0
        || target.key == rent_sponsor.key
        || *system_program_info.key != system_program::id()
    {
        return Err(invalid_light());
    }
    let rent_minimum = Rent::get()
        .map_err(|_| invalid_light())?
        .minimum_balance(account_len);
    let sponsor_bump = [rent_sponsor_bump];
    let sponsor_seeds: &[&[u8]] = &[RENT_SPONSOR_SEED, &sponsor_bump];
    let existing = target.lamports();
    if existing == 0 {
        invoke_system_create_account(
            rent_sponsor,
            target,
            system_program_info,
            rent_minimum,
            account_len,
            program_id,
            &[sponsor_seeds, seeds],
        )
        .map_err(|_| invalid_light())?;
    } else {
        invoke_system_allocate(target, system_program_info, account_len, &[seeds])
            .map_err(|_| invalid_light())?;
        invoke_system_assign(target, system_program_info, program_id, &[seeds])
            .map_err(|_| invalid_light())?;
        let shortfall = rent_minimum.saturating_sub(existing);
        if shortfall > 0 {
            invoke_system_transfer(
                rent_sponsor,
                target,
                system_program_info,
                shortfall,
                &[sponsor_seeds],
            )
            .map_err(|_| invalid_light())?;
        }
    }
    Ok(())
}

impl DecodedAmoebaDlmmState {
    fn create_hot_account<'info>(
        &self,
        program_id: &Pubkey,
        target: &AccountInfo<'info>,
        rent_sponsor: &AccountInfo<'info>,
        system_program_info: &AccountInfo<'info>,
        rent_sponsor_bump: u8,
    ) -> ProgramResult {
        let bump = [self.body[1]];
        match self.kind {
            AmoebaDlmmStateKind::Pool => create_hot_pda(
                program_id,
                target,
                rent_sponsor,
                system_program_info,
                rent_sponsor_bump,
                self.kind.account_len(),
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_POOL_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &bump,
                ],
            ),
            AmoebaDlmmStateKind::BinPage => create_hot_pda(
                program_id,
                target,
                rent_sponsor,
                system_program_info,
                rent_sponsor_bump,
                self.kind.account_len(),
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_BIN_PAGE_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2],
                    &bump,
                ],
            ),
            AmoebaDlmmStateKind::SharePage => create_hot_pda(
                program_id,
                target,
                rent_sponsor,
                system_program_info,
                rent_sponsor_bump,
                self.kind.account_len(),
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_SHARE_PAGE_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[PAGE_INDEX_OFFSET..PAGE_INDEX_OFFSET + 2],
                    &bump,
                ],
            ),
            AmoebaDlmmStateKind::Position => create_hot_pda(
                program_id,
                target,
                rent_sponsor,
                system_program_info,
                rent_sponsor_bump,
                self.kind.account_len(),
                &[
                    CURRENT_STATE_NAMESPACE_SEED,
                    AMOEBA_DLMM_POSITION_PDA_SEED,
                    &self.body[POOL_OR_PAGE_IDENTITY_OFFSET..POOL_OR_PAGE_IDENTITY_OFFSET + 32],
                    &self.body[POSITION_OWNER_OFFSET..POSITION_OWNER_OFFSET + 32],
                    &self.body[POSITION_NONCE_OFFSET..POSITION_NONCE_OFFSET + 8],
                    &bump,
                ],
            ),
        }
    }

    fn write_decompressed(
        &self,
        target: &AccountInfo,
        config: &AmoebaLightConfig,
        slot: u64,
    ) -> ProgramResult {
        let discriminator = self.discriminator();
        if target.data_len() != self.body.len() + 8 {
            return Err(invalid_light());
        }
        let mut data = target.try_borrow_mut_data().map_err(|_| invalid_light())?;
        data[..8].copy_from_slice(&discriminator);
        data[8..].copy_from_slice(&self.body);
        Self::write_compression_info(&mut data[8..], &decompressed_compression_info(config, slot));
        Ok(())
    }
}

pub(super) fn validate_existing_hot(
    program_id: &Pubkey,
    target: &AccountInfo,
    expected: &DecodedAmoebaDlmmState,
) -> Result<bool, ProgramError> {
    if target.owner == program_id {
        let actual = DecodedAmoebaDlmmState::decode(program_id, target)?;
        if !actual.has_layout_with_state(CompressionState::Decompressed)
            || !actual.same_identity(expected)
            || actual.compressed_body() != expected.body
        {
            return Err(invalid_light());
        }
        actual.validate_pda(program_id, target.key)?;
        return Ok(true);
    }
    if target.owner != &system_program::id()
        || target.executable
        || target.data_len() != 0
        || !target.is_writable
    {
        return Err(invalid_light());
    }
    Ok(false)
}

#[inline(never)]
pub(super) fn execute_decompress(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: &DecompressIdempotentParams,
) -> ProgramResult {
    let system_accounts_offset = params.system_accounts_offset as usize;
    let num_pdas = params.token_accounts_offset as usize;
    if accounts.len() < 3 || num_pdas == 0 || num_pdas > params.accounts.len() {
        return Err(invalid_light());
    }
    let hot_start = accounts
        .len()
        .checked_sub(params.accounts.len())
        .ok_or_else(invalid_light)?;
    if system_accounts_offset > hot_start
        || hot_start.saturating_sub(system_accounts_offset) < LIGHT_FIXED_ACCOUNTS
        || !accounts[0].is_signer
        || !accounts[0].is_writable
    {
        return Err(invalid_light());
    }
    let system_accounts = &accounts[system_accounts_offset..hot_start];
    let system_program_info = system_accounts.get(5).ok_or_else(invalid_light)?;
    if *system_program_info.key != system_program::id() {
        return Err(invalid_light());
    }
    let (config, rent_sponsor_bump) =
        load_config_and_sponsor(program_id, &accounts[1], &accounts[2])?;
    let address_tree = config.address_tree;
    let slot = Clock::get()?.slot;
    let mut account_infos = Vec::with_capacity(num_pdas);

    for (packed, target) in params.accounts[..num_pdas]
        .iter()
        .zip(&accounts[hot_start..hot_start + num_pdas])
    {
        let state = &packed.data;
        if !state.has_layout_with_state(CompressionState::Compressed)
            || state.compression_info()? != CompressionInfo::compressed()
        {
            return Err(invalid_light());
        }
        state.validate_pda(program_id, target.key)?;
        if validate_existing_hot(program_id, target, state)? {
            continue;
        }

        let address = derive_assigned_address(
            &target.key.to_bytes(),
            &address_tree,
            &program_id.to_bytes(),
        );
        state.create_hot_account(
            program_id,
            target,
            &accounts[2],
            system_program_info,
            rent_sponsor_bump,
        )?;
        state.write_decompressed(target, &config, slot)?;

        account_infos.push(CompressedAccountInfo {
            address: Some(address),
            input: Some(InAccountInfo {
                discriminator: state.discriminator(),
                data_hash: hash_leaf_data(&state.body).map_err(|_| invalid_light())?,
                merkle_context: PackedMerkleContext {
                    merkle_tree_pubkey_index: packed.tree_info.merkle_tree_pubkey_index,
                    queue_pubkey_index: packed.tree_info.queue_pubkey_index,
                    leaf_index: packed.tree_info.leaf_index,
                    prove_by_index: packed.tree_info.prove_by_index,
                },
                root_index: if packed.tree_info.prove_by_index {
                    0
                } else {
                    packed.tree_info.root_index
                },
                lamports: 0,
            }),
            output: Some(OutAccountInfo {
                discriminator: DECOMPRESSED_PDA_DISCRIMINATOR,
                data_hash: hash_leaf_data(target.key.as_ref()).map_err(|_| invalid_light())?,
                output_merkle_tree_index: params.output_queue_index,
                lamports: 0,
                data: target.key.to_bytes().to_vec(),
            }),
        });
    }

    if account_infos.is_empty() {
        return Ok(());
    }
    let mut instruction = new_light_system_cpi(params.proof);
    instruction.account_infos = account_infos;
    invoke_light_cpi(instruction, &accounts[0], system_accounts).map_err(|_| invalid_light())
}

#[inline(never)]
pub(super) fn process_decompress(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    payload: &[u8],
) -> ProgramResult {
    let params = decode_decompress_params(payload)?;
    execute_decompress(program_id, accounts, &params)
}
