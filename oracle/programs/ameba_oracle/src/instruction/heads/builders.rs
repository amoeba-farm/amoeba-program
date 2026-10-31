//! Client builders of tag 210 with their canonical account lists. Every builder that creates an
//! account takes the rent `payer` right after `program_id` (it may be the signer's own key; a
//! sponsor otherwise): accounts that can close record it and return their rent to it, never to
//! their owner. Ops, the API, petri and the
//! chain call these; the sender appends the governance envelope where the cluster's build has
//! one. Accounts marked "placeholder" take the system program id when the role does not apply,
//! so every selector has one fixed list.

use super::*;
use crate::state::heads::{
    find_head_address, find_head_challenge_address, find_head_claim_address,
    find_head_delegation_address, find_heads_amba_vault_address, find_heads_authority_address,
    find_heads_board_address, find_heads_config_address, find_heads_contract_address,
    find_heads_epoch_address, find_heads_stake_address, find_heads_weights_address,
};
use crate::state::{derive_user_collateral_pda, find_bond_vault_address, find_config_address};
use ameba_oracle_interface::find_period_address;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk_ids::system_program;

fn heads_ix(
    program_id: &Pubkey,
    instruction: HeadsInstruction,
    accounts: Vec<AccountMeta>,
) -> Result<Instruction, crate::ProgramError> {
    let data = instruction.data().map_err(|_| {
        crate::ProgramError::from(crate::error::OracleError::InvalidInstructionData)
    })?;
    Ok(Instruction {
        program_id: *program_id,
        accounts,
        data,
    })
}

/// For payloads that cannot fail to encode.
fn infallible(result: Result<Instruction, crate::ProgramError>) -> Instruction {
    result.expect("a fixed heads payload always encodes")
}

fn w(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}

fn r(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}

fn ws(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, true)
}

fn rs(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}

fn config(program_id: &Pubkey) -> Pubkey {
    find_config_address(program_id).0
}

fn heads_config(program_id: &Pubkey) -> Pubkey {
    find_heads_config_address(program_id).0
}

fn collateral(program_id: &Pubkey, owner: &Pubkey) -> Pubkey {
    derive_user_collateral_pda(program_id, owner).0
}

fn stake_of(program_id: &Pubkey, owner: &Pubkey) -> Pubkey {
    find_heads_stake_address(program_id, owner).0
}

fn evidence_or_placeholder(object: Option<&Pubkey>) -> Pubkey {
    object.copied().unwrap_or(system_program::ID)
}

/// The addresses one head's instructions use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadAddresses {
    pub board: Pubkey,
    pub head: Pubkey,
}

impl HeadAddresses {
    pub fn derive(program_id: &Pubkey, product_id: &[u8; 32], head_id: u32) -> Self {
        let board = find_heads_board_address(program_id, product_id).0;
        Self {
            board,
            head: find_head_address(program_id, &board, head_id).0,
        }
    }

    pub fn delegation(&self, program_id: &Pubkey, owner: &Pubkey) -> Pubkey {
        find_head_delegation_address(program_id, &self.head, owner).0
    }

    pub fn claim(&self, program_id: &Pubkey, seq: u32) -> Pubkey {
        find_head_claim_address(program_id, &self.head, seq).0
    }
}

/// The addresses one contract's instructions use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractAddresses {
    pub board: Pubkey,
    pub period: Pubkey,
    pub contract: Pubkey,
}

impl ContractAddresses {
    pub fn derive(program_id: &Pubkey, product_id: &[u8; 32], expiry_ts: u64) -> Self {
        let period = find_period_address(program_id, product_id, expiry_ts).0;
        Self {
            board: find_heads_board_address(program_id, product_id).0,
            period,
            contract: find_heads_contract_address(program_id, &period).0,
        }
    }

    pub fn weights(&self, program_id: &Pubkey, component: u8) -> Pubkey {
        find_heads_weights_address(program_id, &self.contract, component).0
    }
}

/// Selector 0. Accounts: 0 [signer, writable] payer; 1 [signer] config admin; 2 oracle config;
/// 3 [writable] heads config PDA; 4 heads-authority PDA; 5 AMBA mint; 6 AMBA vault (the
/// authority's associated token account, created beforehand by the client); 7 AMBA token
/// program; 8 system program.
pub fn init_heads_config(
    program_id: &Pubkey,
    payer: &Pubkey,
    admin: &Pubkey,
    amba_mint: &Pubkey,
    amba_token_program: &Pubkey,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::InitHeadsConfig(InitHeadsConfigParams {
            amba_mint: *amba_mint,
            amba_token_program: *amba_token_program,
        }),
        vec![
            ws(*payer),
            rs(*admin),
            r(config(program_id)),
            w(heads_config(program_id)),
            r(find_heads_authority_address(program_id).0),
            r(*amba_mint),
            r(find_heads_amba_vault_address(
                program_id,
                amba_mint,
                amba_token_program,
            )),
            r(*amba_token_program),
            r(system_program::ID),
        ],
    ))
}

/// Selector 1. Accounts: 0 [signer, writable] payer; 1 [signer] config admin; 2 oracle config;
/// 3 [writable] heads config; 4 [writable] board PDA; 5 system program. Fails to build with no
/// items, more than `HEADS_MAX_ITEMS`, or a mix that does not add up to 10,000. Each item brings
/// its three components; [`set_component`] sets their sentences.
pub fn init_board(
    program_id: &Pubkey,
    payer: &Pubkey,
    admin: &Pubkey,
    product_id: [u8; 32],
    params: HeadsParams,
    items: Vec<HeadsItemParams>,
) -> Result<Instruction, crate::ProgramError> {
    let board = find_heads_board_address(program_id, &product_id).0;
    heads_ix(
        program_id,
        HeadsInstruction::InitBoard(InitBoardParams {
            product_id,
            params,
            items,
        }),
        vec![
            ws(*payer),
            rs(*admin),
            r(config(program_id)),
            w(heads_config(program_id)),
            w(board),
            r(system_program::ID),
        ],
    )
}

/// Selector 2. Accounts: 0 [signer] config admin; 1 oracle config; 2 [writable] board.
/// Selector 31. Accounts: 0 [signer] admin; 1 oracle config; 2 [writable] board.
pub fn set_ruling_deadline(
    program_id: &Pubkey,
    admin: &Pubkey,
    product_id: &[u8; 32],
    secs: u64,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::SetRulingDeadline { secs },
        vec![
            rs(*admin),
            r(config(program_id)),
            w(find_heads_board_address(program_id, product_id).0),
        ],
    ))
}

pub fn set_board_params(
    program_id: &Pubkey,
    admin: &Pubkey,
    product_id: &[u8; 32],
    expected_params_version: u32,
    params: HeadsParams,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::SetBoardParams(SetBoardParamsParams {
            expected_params_version,
            params,
        }),
        vec![
            rs(*admin),
            r(config(program_id)),
            w(find_heads_board_address(program_id, product_id).0),
        ],
    ))
}

/// Selector 26: set item `index`, or append it at the board's `item_count`. Accounts:
/// 0 [signer] config admin; 1 oracle config; 2 [writable] board. Fails to build with an index out
/// of range or a mix that does not add up to 10,000.
pub fn set_item(
    program_id: &Pubkey,
    admin: &Pubkey,
    product_id: &[u8; 32],
    index: u8,
    item: HeadsItemParams,
) -> Result<Instruction, crate::ProgramError> {
    heads_ix(
        program_id,
        HeadsInstruction::SetItem(SetItemParams { index, item }),
        vec![
            rs(*admin),
            r(config(program_id)),
            w(find_heads_board_address(program_id, product_id).0),
        ],
    )
}

/// Selector 27: set the sentence of component `index` (`heads_component_index(item, channel)`,
/// the mold's `item * 3 + channel`). Accounts: 0 [signer] config admin; 1 oracle config;
/// 2 [writable] board; 3 the sealed kind-2 object holding the sentence (`sentence_hash` is its
/// commitment). Fails to build when the index is not the mold's for `(item, channel)`.
#[allow(clippy::too_many_arguments)]
pub fn set_component(
    program_id: &Pubkey,
    admin: &Pubkey,
    product_id: &[u8; 32],
    index: u8,
    item: u8,
    channel: HeadsChannel,
    sentence_hash: [u8; 32],
    sentence_object: &Pubkey,
) -> Result<Instruction, crate::ProgramError> {
    heads_ix(
        program_id,
        HeadsInstruction::SetComponent(SetComponentParams {
            index,
            item,
            channel,
            sentence_hash,
        }),
        vec![
            rs(*admin),
            r(config(program_id)),
            w(find_heads_board_address(program_id, product_id).0),
            r(*sentence_object),
        ],
    )
}

/// Selector 3. `month_id` is `ameba_oracle_heads::time::month_id(now)`, the prize epoch the cut
/// goes to. Accounts: 0 [signer, writable] payer (the epoch's rent when it is new); 1 [signer]
/// funder; 2 oracle config; 3 [writable] funder's `UserCollateral`; 4 [writable] board;
/// 5 [writable] epoch PDA (board, month_id); 6 system program.
pub fn fund_board(
    program_id: &Pubkey,
    payer: &Pubkey,
    funder: &Pubkey,
    product_id: &[u8; 32],
    month_id: u32,
    amount: u64,
) -> Instruction {
    let board = find_heads_board_address(program_id, product_id).0;
    infallible(heads_ix(
        program_id,
        HeadsInstruction::FundBoard { amount },
        vec![
            ws(*payer),
            rs(*funder),
            r(config(program_id)),
            w(collateral(program_id, funder)),
            w(board),
            w(find_heads_epoch_address(program_id, &board, month_id).0),
            r(system_program::ID),
        ],
    ))
}

/// Selector 4. Accounts: 0 [signer, writable] payer (the stake account's rent when it is new; a
/// stake never closes); 1 [signer] owner; 2 [writable] heads config; 3 [writable] owner's stake
/// PDA; 4 [writable] the owner's AMBA token account (source); 5 [writable] AMBA vault; 6 AMBA
/// mint; 7 AMBA token program; 8 system program.
#[allow(clippy::too_many_arguments)]
pub fn stake(
    program_id: &Pubkey,
    payer: &Pubkey,
    owner: &Pubkey,
    source_token_account: &Pubkey,
    amba_mint: &Pubkey,
    amba_token_program: &Pubkey,
    amount: u64,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::Stake { amount },
        vec![
            ws(*payer),
            rs(*owner),
            w(heads_config(program_id)),
            w(stake_of(program_id, owner)),
            w(*source_token_account),
            w(find_heads_amba_vault_address(
                program_id,
                amba_mint,
                amba_token_program,
            )),
            r(*amba_mint),
            r(*amba_token_program),
            r(system_program::ID),
        ],
    ))
}

/// Selector 5. Accounts: 0 [signer] owner; 1 [writable] heads config; 2 [writable] owner's
/// stake; 3 heads-authority PDA; 4 [writable] AMBA vault; 5 [writable] destination AMBA token
/// account; 6 AMBA mint; 7 AMBA token program.
pub fn unstake(
    program_id: &Pubkey,
    owner: &Pubkey,
    destination_token_account: &Pubkey,
    amba_mint: &Pubkey,
    amba_token_program: &Pubkey,
    amount: u64,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::Unstake { amount },
        vec![
            rs(*owner),
            w(heads_config(program_id)),
            w(stake_of(program_id, owner)),
            r(find_heads_authority_address(program_id).0),
            w(find_heads_amba_vault_address(
                program_id,
                amba_mint,
                amba_token_program,
            )),
            w(*destination_token_account),
            r(*amba_mint),
            r(*amba_token_program),
        ],
    ))
}

/// Selector 6. `head_id` is the board's current `head_count`. Accounts: 0 [signer, writable]
/// payer (the head's and the delegation's rent; recorded on the delegation, which returns it
/// when it closes); 1 [signer] proposer; 2 [writable] heads config; 3 [writable] board;
/// 4 [writable] proposer's stake; 5 [writable] head PDA; 6 [writable] proposer's delegation PDA;
/// 7 the sealed kind-2 spec object; 8 system program; 9 [writable] proposer's `UserCollateral`
/// (created empty, the payer funding it, when it does not exist: a standing source case pays
/// the proposer there). Fails to build with a name over `HEAD_NAME_MAX_BYTES` or a component
/// index out of range.
#[allow(clippy::too_many_arguments)]
pub fn post_head(
    program_id: &Pubkey,
    payer: &Pubkey,
    proposer: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    component: u8,
    access_class: HeadAccessClass,
    amount: u64,
    name: &str,
    spec_hash: [u8; 32],
    spec_object: &Pubkey,
) -> Result<Instruction, crate::ProgramError> {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    heads_ix(
        program_id,
        HeadsInstruction::PostHead(PostHeadParams {
            expected_head_id: head_id,
            component,
            access_class,
            amount,
            spec_hash,
            name: name.to_owned(),
        }),
        vec![
            ws(*payer),
            rs(*proposer),
            w(heads_config(program_id)),
            w(a.board),
            w(stake_of(program_id, proposer)),
            w(a.head),
            w(a.delegation(program_id, proposer)),
            r(*spec_object),
            r(system_program::ID),
            w(collateral(program_id, proposer)),
        ],
    )
}

/// Selector 7. Accounts: 0 [signer, writable] payer (the delegation's rent when it is new,
/// recorded and returned to it when the delegation closes); 1 [signer] owner; 2 [writable] heads
/// config; 3 [writable] board; 4 [writable] head; 5 [writable] owner's stake; 6 [writable]
/// owner's delegation PDA; 7 system program.
pub fn delegate(
    program_id: &Pubkey,
    payer: &Pubkey,
    owner: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    amount: u64,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::Delegate { amount },
        vec![
            ws(*payer),
            rs(*owner),
            w(heads_config(program_id)),
            w(a.board),
            w(a.head),
            w(stake_of(program_id, owner)),
            w(a.delegation(program_id, owner)),
            r(system_program::ID),
        ],
    ))
}

/// Selector 8. Accounts: 0 [signer] owner; 1 [writable] heads config; 2 [writable] board;
/// 3 [writable] head; 4 [writable] owner's stake; 5 [writable] owner's delegation.
pub fn undelegate(
    program_id: &Pubkey,
    owner: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    amount: u64,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::Undelegate { amount },
        vec![
            rs(*owner),
            w(heads_config(program_id)),
            w(a.board),
            w(a.head),
            w(stake_of(program_id, owner)),
            w(a.delegation(program_id, owner)),
        ],
    ))
}

/// Selector 9, permissionless. `rent_payer` is the delegation's recorded `rent_payer` (its owner
/// for a delegation from before the field). Accounts: 0 [writable] heads config; 1 [writable]
/// board; 2 [writable] head; 3 [writable] the owner's stake; 4 [writable] the owner's
/// delegation; 5 [writable] the rent payer (receives the rent when the delegation closes).
pub fn sync_delegation(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    owner: &Pubkey,
    rent_payer: &Pubkey,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::SyncDelegation,
        vec![
            w(heads_config(program_id)),
            w(a.board),
            w(a.head),
            w(stake_of(program_id, owner)),
            w(a.delegation(program_id, owner)),
            w(*rent_payer),
        ],
    ))
}

/// Selector 10, permissionless. Accounts: 0 [writable] board; 1 [writable] head.
pub fn expire_head(program_id: &Pubkey, product_id: &[u8; 32], head_id: u32) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::ExpireHead,
        vec![w(a.board), w(a.head)],
    ))
}

/// Selector 11: anyone fills an Active head or updates a Live one. `seq` is the head's current
/// `claim_seq`; `commit_hash` is [`heads_claim_commitment`]. Accounts: 0 [signer, writable] payer
/// (the claim's rent, recorded); 1 [signer] fetcher; 2 oracle config; 3 board; 4 [writable] head;
/// 5 [writable] claim PDA; 6 [writable] fetcher's `UserCollateral`; 7 system program.
#[allow(clippy::too_many_arguments)]
pub fn commit_claim(
    program_id: &Pubkey,
    payer: &Pubkey,
    fetcher: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    kind: HeadClaimKind,
    commit_hash: [u8; 32],
    bond: u64,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::CommitClaim(CommitClaimParams {
            expected_seq: seq,
            kind,
            commit_hash,
            bond,
        }),
        vec![
            ws(*payer),
            rs(*fetcher),
            r(config(program_id)),
            r(a.board),
            w(a.head),
            w(a.claim(program_id, seq)),
            w(collateral(program_id, fetcher)),
            r(system_program::ID),
        ],
    ))
}

/// Selector 12. `reveal` must reproduce the commit's [`heads_claim_commitment`];
/// `evidence_object` is the sealed object `reveal.evidence_kind` names (`None` for kind 0).
/// Accounts: 0 [signer] fetcher; 1 board; 2 [writable] head; 3 [writable] claim; 4 the evidence
/// object (placeholder: the system program).
pub fn reveal_claim(
    program_id: &Pubkey,
    fetcher: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    reveal: RevealClaimParams,
    evidence_object: Option<&Pubkey>,
) -> Result<Instruction, crate::ProgramError> {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    heads_ix(
        program_id,
        HeadsInstruction::RevealClaim(reveal),
        vec![
            rs(*fetcher),
            r(a.board),
            w(a.head),
            w(a.claim(program_id, seq)),
            r(evidence_or_placeholder(evidence_object)),
        ],
    )
}

/// Selector 13. Accounts: 0 [signer, writable] payer (the challenge's rent, recorded and returned
/// when it settles); 1 [signer] challenger; 2 oracle config; 3 board; 4 [writable] head;
/// 5 [writable] claim; 6 [writable] challenge PDA (claim, challenger); 7 [writable] challenger's
/// `UserCollateral`; 8 the evidence object (placeholder: the system program); 9 system program.
#[allow(clippy::too_many_arguments)]
pub fn challenge_claim(
    program_id: &Pubkey,
    payer: &Pubkey,
    challenger: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    params: ChallengeClaimParams,
    evidence_object: Option<&Pubkey>,
) -> Result<Instruction, crate::ProgramError> {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    let claim = a.claim(program_id, seq);
    heads_ix(
        program_id,
        HeadsInstruction::ChallengeClaim(params),
        vec![
            ws(*payer),
            rs(*challenger),
            r(config(program_id)),
            r(a.board),
            w(a.head),
            w(claim),
            w(find_head_challenge_address(program_id, &claim, challenger).0),
            w(collateral(program_id, challenger)),
            r(evidence_or_placeholder(evidence_object)),
            r(system_program::ID),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn rule_challenge(
    program_id: &Pubkey,
    council_config: &Pubkey,
    seats: &[Pubkey; 3],
    board: Pubkey,
    head: Pubkey,
    challenge: Pubkey,
    claim: Option<Pubkey>,
    params: RuleChallengeParams,
) -> Instruction {
    let mut accounts = vec![r(*council_config)];
    accounts.extend(seats.iter().map(|seat| rs(*seat)));
    accounts.push(w(head));
    accounts.push(w(challenge));
    accounts.push(match claim {
        Some(claim) => w(claim),
        None => r(system_program::ID),
    });
    accounts.push(r(board));
    infallible(heads_ix(
        program_id,
        HeadsInstruction::RuleChallenge(params),
        accounts,
    ))
}

/// Selector 14 for a claim challenge. Accounts: 0 the council Config (as tag 130/0 reads it);
/// 1-3 [signer] three distinct current council seats; 4 [writable] head; 5 [writable]
/// challenge; 6 [writable] claim; 7 the board (which contracts admit the claim once ruled).
#[allow(clippy::too_many_arguments)]
pub fn rule_claim_challenge(
    program_id: &Pubkey,
    council_config: &Pubkey,
    seats: &[Pubkey; 3],
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    challenger: &Pubkey,
    won: bool,
    expected_epoch: u64,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    let claim = a.claim(program_id, seq);
    rule_challenge(
        program_id,
        council_config,
        seats,
        a.board,
        a.head,
        find_head_challenge_address(program_id, &claim, challenger).0,
        Some(claim),
        RuleChallengeParams {
            kind: HeadChallengeKind::Claim,
            ruling: if won {
                HeadChallengeRuling::Won
            } else {
                HeadChallengeRuling::Lost
            },
            expected_epoch,
        },
    )
}

/// Selector 14 for a source challenge. Accounts: 0 the council Config; 1-3 [signer] three
/// distinct current council seats; 4 [writable] head; 5 [writable] challenge; 6 placeholder
/// (the system program); 7 the board.
#[allow(clippy::too_many_arguments)]
pub fn rule_source_challenge(
    program_id: &Pubkey,
    council_config: &Pubkey,
    seats: &[Pubkey; 3],
    product_id: &[u8; 32],
    head_id: u32,
    challenger: &Pubkey,
    won: bool,
    expected_epoch: u64,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    rule_challenge(
        program_id,
        council_config,
        seats,
        a.board,
        a.head,
        find_head_challenge_address(program_id, &a.head, challenger).0,
        None,
        RuleChallengeParams {
            kind: HeadChallengeKind::Source,
            ruling: if won {
                HeadChallengeRuling::Won
            } else {
                HeadChallengeRuling::Lost
            },
            expected_epoch,
        },
    )
}

/// Selector 15, permissionless. `month_id` is `time::month_id(now)`, the epoch a standing
/// claim's prize points go to. Accounts: 0 [signer, writable] payer (the epoch's rent when it
/// is new); 1 oracle config; 2 [writable] board; 3 [writable] head; 4 [writable] claim;
/// 5 [writable] the fetcher's `UserCollateral`; 6 [writable] epoch PDA; 7 system program.
pub fn finalize_claim(
    program_id: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    fetcher: &Pubkey,
    month_id: u32,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::FinalizeClaim,
        vec![
            ws(*payer),
            r(config(program_id)),
            w(a.board),
            w(a.head),
            w(a.claim(program_id, seq)),
            w(collateral(program_id, fetcher)),
            w(find_heads_epoch_address(program_id, &a.board, month_id).0),
            r(system_program::ID),
        ],
    ))
}

fn settle_challenge(
    program_id: &Pubkey,
    head: Pubkey,
    challenge: Pubkey,
    claim: Option<Pubkey>,
    challenger: &Pubkey,
    rent_payer: &Pubkey,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::SettleChallenge,
        vec![
            r(config(program_id)),
            w(heads_config(program_id)),
            w(head),
            w(challenge),
            match claim {
                Some(claim) => w(claim),
                None => r(system_program::ID),
            },
            w(collateral(program_id, challenger)),
            w(stake_of(program_id, challenger)),
            w(*rent_payer),
        ],
    ))
}

/// Selector 16 for a claim challenge, permissionless. Accounts: 0 oracle config; 1 [writable]
/// heads config; 2 [writable] head; 3 [writable] challenge; 4 [writable] claim; 5 [writable]
/// challenger's `UserCollateral`; 6 [writable] challenger's stake PDA (unused for a claim
/// challenge); 7 [writable] the challenge's recorded `rent_payer` (its challenger for a challenge
/// from before the field), which receives the rent when it closes.
pub fn settle_claim_challenge(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    challenger: &Pubkey,
    rent_payer: &Pubkey,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    let claim = a.claim(program_id, seq);
    settle_challenge(
        program_id,
        a.head,
        find_head_challenge_address(program_id, &claim, challenger).0,
        Some(claim),
        challenger,
        rent_payer,
    )
}

/// Selector 16 for a source challenge, permissionless. Accounts as for
/// [`settle_claim_challenge`], with 4 a placeholder (the system program); a won source
/// challenge's AMBA award goes to 6, the challenger's stake (which must exist).
pub fn settle_source_challenge(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    challenger: &Pubkey,
    rent_payer: &Pubkey,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    settle_challenge(
        program_id,
        a.head,
        find_head_challenge_address(program_id, &a.head, challenger).0,
        None,
        challenger,
        rent_payer,
    )
}

/// Selector 17, permissionless. Accounts: 0 oracle config; 1 board; 2 [writable] head;
/// 3 [writable] claim; 4 [writable] the fetcher's `UserCollateral`.
pub fn refund_expired_commit(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    fetcher: &Pubkey,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::RefundExpiredCommit,
        vec![
            r(config(program_id)),
            r(a.board),
            w(a.head),
            w(a.claim(program_id, seq)),
            w(collateral(program_id, fetcher)),
        ],
    ))
}

/// Selector 18. Accounts: 0 [signer, writable] payer (the challenge's rent, recorded and returned
/// when it settles); 1 [signer] challenger; 2 oracle config; 3 board; 4 [writable] head;
/// 5 [writable] challenge PDA (head, challenger); 6 [writable] challenger's `UserCollateral`;
/// 7 the evidence object (placeholder: the system program); 8 system program; 9 [writable]
/// challenger's stake (created, the payer funding it, when it does not exist: a winner is paid
/// AMBA there).
#[allow(clippy::too_many_arguments)]
pub fn challenge_source(
    program_id: &Pubkey,
    payer: &Pubkey,
    challenger: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    params: ChallengeSourceParams,
    evidence_object: Option<&Pubkey>,
) -> Result<Instruction, crate::ProgramError> {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    heads_ix(
        program_id,
        HeadsInstruction::ChallengeSource(params),
        vec![
            ws(*payer),
            rs(*challenger),
            r(config(program_id)),
            r(a.board),
            w(a.head),
            w(find_head_challenge_address(program_id, &a.head, challenger).0),
            w(collateral(program_id, challenger)),
            r(evidence_or_placeholder(evidence_object)),
            r(system_program::ID),
            w(stake_of(program_id, challenger)),
        ],
    )
}

/// Selector 19, permissionless. Accounts: 0 oracle config; 1 [writable] heads config;
/// 2 [writable] board; 3 [writable] head; 4 [writable] the proposer's delegation; 5 [writable]
/// the proposer's `UserCollateral` (challengers' bonds when the source stands; unused when it
/// falls).
pub fn finalize_source_challenges(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    proposer: &Pubkey,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::FinalizeSourceChallenges,
        vec![
            r(config(program_id)),
            w(heads_config(program_id)),
            w(a.board),
            w(a.head),
            w(a.delegation(program_id, proposer)),
            w(collateral(program_id, proposer)),
        ],
    ))
}

/// Selector 20. Accounts: 0 [signer, writable] payer (the contract's rent); 1 oracle config;
/// 2 [signer] oracle authority; 3 [writable] the Forming period; 4 the period's v1 month PDA
/// (must not exist); 5 [writable] board; 6 [writable] contract PDA; 7 the sealed kind-2 listing
/// terms object (`terms_hash` is its commitment; it carries the `mixCommitment` of `items`, the
/// board's items' mixes for the contract's quarter: `commodity-spec`'s `listing_mixes`);
/// 8 system program. The components' weights accounts are created later, by their first
/// `SettleComponent`. Fails to build with no items, more than `HEADS_MAX_ITEMS`, an invalid mix
/// or a zero `terms_hash`.
#[allow(clippy::too_many_arguments)]
pub fn list_contract(
    program_id: &Pubkey,
    payer: &Pubkey,
    oracle_authority: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
    base: u64,
    settlement_target: HeadsSettlementTarget,
    items: &[HeadsListedMix],
    terms_hash: [u8; 32],
    terms_object: &Pubkey,
) -> Result<Instruction, crate::ProgramError> {
    let c = ContractAddresses::derive(program_id, product_id, expiry_ts);
    heads_ix(
        program_id,
        HeadsInstruction::ListContract(ListContractParams {
            base,
            settlement_target,
            terms_hash,
            items: items.to_vec(),
        }),
        vec![
            ws(*payer),
            r(config(program_id)),
            rs(*oracle_authority),
            w(c.period),
            r(crate::instruction::find_oracle_month_address(
                program_id, &c.period, expiry_ts,
            )),
            w(c.board),
            w(c.contract),
            r(*terms_object),
            r(system_program::ID),
        ],
    )
}

/// Selector 28: a Forming period of `product_id` expiring at a calendar-quarter boundary,
/// independent of v1's monthly ladder. `planned_listing_ts` is at or before the quarter's start,
/// or at or before now (the current quarter lists at once), and before the expiry. Accounts:
/// 0 [signer, writable] payer (the period's rent); 1 oracle config; 2 [signer] oracle authority;
/// 3 the product's heads board; 4 [writable] period PDA (product, expiry); 5 system program. Fails
/// to build when the expiry is no quarter boundary or the listing is not before it.
pub fn create_quarter_period(
    program_id: &Pubkey,
    payer: &Pubkey,
    oracle_authority: &Pubkey,
    product_id: &[u8; 32],
    planned_listing_ts: u64,
    expiry_ts: u64,
) -> Result<Instruction, crate::ProgramError> {
    let c = ContractAddresses::derive(program_id, product_id, expiry_ts);
    heads_ix(
        program_id,
        HeadsInstruction::CreateQuarterPeriod(CreateQuarterPeriodParams {
            product_id: *product_id,
            planned_listing_ts,
            expiry_ts,
        }),
        vec![
            ws(*payer),
            r(config(program_id)),
            rs(*oracle_authority),
            r(c.board),
            w(c.period),
            r(system_program::ID),
        ],
    )
}

/// Selector 21, permissionless. `source_head_ids` are the component's sources from
/// `first_source` on, in the weights' source order (the first call creates the weights with the
/// contract's listed sources; append a source that went live in the window by passing its head
/// after the recorded ones, in any order: each is recorded once). Accounts: 0 [signer, writable]
/// payer (the weights' rent and growth); 1 board; 2 [writable] contract; 3 [writable] the
/// component's weights; 4 system program; then 5.. [writable] each of those heads (synced, and
/// marked recorded).
pub fn settle_component(
    program_id: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
    component: u8,
    first_source: u16,
    source_head_ids: &[u32],
) -> Result<Instruction, crate::ProgramError> {
    let c = ContractAddresses::derive(program_id, product_id, expiry_ts);
    let mut accounts = vec![
        ws(*payer),
        r(c.board),
        w(c.contract),
        w(c.weights(program_id, component)),
        r(system_program::ID),
    ];
    accounts.extend(
        source_head_ids
            .iter()
            .map(|id| w(find_head_address(program_id, &c.board, *id).0)),
    );
    heads_ix(
        program_id,
        HeadsInstruction::SettleComponent(SettleComponentParams {
            component,
            first_source,
        }),
        accounts,
    )
}

/// Selector 22, permissionless. Accounts: 0 [writable] board; 1 [writable] contract;
/// 2 [writable] period.
pub fn finalize_contract(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    expiry_ts: u64,
) -> Instruction {
    let c = ContractAddresses::derive(program_id, product_id, expiry_ts);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::FinalizeContract,
        vec![w(c.board), w(c.contract), w(c.period)],
    ))
}

/// Selector 23. Accounts: 0 [signer] owner; 1 oracle config; 2 [writable] board; 3 head;
/// 4 [writable] owner's delegation; 5 [writable] owner's `UserCollateral`.
pub fn claim_rewards(
    program_id: &Pubkey,
    owner: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::ClaimRewards,
        vec![
            rs(*owner),
            r(config(program_id)),
            w(a.board),
            r(a.head),
            w(a.delegation(program_id, owner)),
            w(collateral(program_id, owner)),
        ],
    ))
}

/// Selector 24. `month_id` is the claim's `epoch`. Accounts: 0 [signer] fetcher; 1 oracle
/// config; 2 [writable] board; 3 head; 4 [writable] claim; 5 [writable] epoch; 6 [writable]
/// fetcher's `UserCollateral`.
/// Selector 29, permissionless. Accounts: 0 [signer, writable] payer (the current month's epoch
/// when it is new); 1 board; 2 [writable] the ended month's epoch; 3 [writable] the current
/// month's epoch (`into_month`, `time::month_id` of now); 4 system program.
pub fn roll_over_prizes(
    program_id: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    ended_month: u32,
    into_month: u32,
) -> Instruction {
    let board = find_heads_board_address(program_id, product_id).0;
    infallible(heads_ix(
        program_id,
        HeadsInstruction::RollOverPrizes,
        vec![
            ws(*payer),
            r(board),
            w(find_heads_epoch_address(program_id, &board, ended_month).0),
            w(find_heads_epoch_address(program_id, &board, into_month).0),
            r(system_program::ID),
        ],
    ))
}

/// Selector 30, permissionless, dev builds only (`heads-migrate`), on a round-6 head (version
/// 2) carrying a renewal join, a left-out mark or a prize day before its floor: no contracts and
/// no claims. Accounts: 0 [signer, writable] payer; 1 [writable] board; 2 [writable] head;
/// 3 system program.
pub fn migrate_head(
    program_id: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    contracts: &[Pubkey],
    open_claims: &[Pubkey],
) -> Result<Instruction, crate::ProgramError> {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    let count = u8::try_from(contracts.len()).map_err(|_| crate::ProgramError::InvalidArgument)?;
    let mut accounts = vec![ws(*payer), w(a.board), w(a.head), r(system_program::ID)];
    accounts.extend(contracts.iter().map(|k| r(*k)));
    accounts.extend(open_claims.iter().map(|k| w(*k)));
    heads_ix(
        program_id,
        HeadsInstruction::MigrateHead { contracts: count },
        accounts,
    )
}

/// Selector 30 on a version-1 board (dev builds only): its live entries' delegators and
/// memberships, from `heads` (every live head of the board) and `contracts` (every unsettled
/// ring contract). Accounts: 0 [signer, writable] payer; 1 [writable] board; 2 [writable] the
/// board again; 3 system program; then the heads, then the contracts.
pub fn migrate_board(
    program_id: &Pubkey,
    payer: &Pubkey,
    product_id: &[u8; 32],
    heads: &[Pubkey],
    contracts: &[Pubkey],
) -> Result<Instruction, crate::ProgramError> {
    let board = find_heads_board_address(program_id, product_id).0;
    let count = u8::try_from(contracts.len()).map_err(|_| crate::ProgramError::InvalidArgument)?;
    let mut accounts = vec![ws(*payer), w(board), w(board), r(system_program::ID)];
    accounts.extend(heads.iter().map(|k| r(*k)));
    accounts.extend(contracts.iter().map(|k| r(*k)));
    heads_ix(
        program_id,
        HeadsInstruction::MigrateHead { contracts: count },
        accounts,
    )
}

pub fn claim_prize(
    program_id: &Pubkey,
    fetcher: &Pubkey,
    product_id: &[u8; 32],
    head_id: u32,
    seq: u32,
    month_id: u32,
) -> Instruction {
    let a = HeadAddresses::derive(program_id, product_id, head_id);
    infallible(heads_ix(
        program_id,
        HeadsInstruction::ClaimPrize,
        vec![
            rs(*fetcher),
            r(config(program_id)),
            w(a.board),
            r(a.head),
            w(a.claim(program_id, seq)),
            w(find_heads_epoch_address(program_id, &a.board, month_id).0),
            w(collateral(program_id, fetcher)),
        ],
    ))
}

/// Selector 25, permissionless. Accounts: 0 [writable] heads config; 1 heads-authority PDA;
/// 2 [writable] AMBA vault; 3 [writable] AMBA mint; 4 AMBA token program; 5 [writable] board;
/// 6 oracle config (the bond vault's authority); 7 [writable] bond vault; 8 [writable] USDC
/// mint; 9 classic SPL Token program.
pub fn burn_pending(
    program_id: &Pubkey,
    product_id: &[u8; 32],
    amba_mint: &Pubkey,
    amba_token_program: &Pubkey,
    usdc_mint: &Pubkey,
) -> Instruction {
    infallible(heads_ix(
        program_id,
        HeadsInstruction::BurnPending,
        vec![
            w(heads_config(program_id)),
            r(find_heads_authority_address(program_id).0),
            w(find_heads_amba_vault_address(
                program_id,
                amba_mint,
                amba_token_program,
            )),
            w(*amba_mint),
            r(*amba_token_program),
            w(find_heads_board_address(program_id, product_id).0),
            r(config(program_id)),
            w(find_bond_vault_address(program_id, usdc_mint)),
            w(*usdc_mint),
            r(crate::token_instruction::id()),
        ],
    ))
}
