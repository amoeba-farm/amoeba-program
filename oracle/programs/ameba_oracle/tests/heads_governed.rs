//! Tag 210 under the governance envelope: the dev chain's `localnet-governance` profile and the
//! chain's production profile with `heads` route it only through the controller's gate, which
//! strips its tail first; without the envelope it is refused before any account is read.
#![cfg(all(feature = "heads", not(feature = "localnet-dev-authority")))]

mod common;

use ameba_oracle::{
    error::OracleError,
    governance_gate::{
        derive_protocol_gate_pda, GovernanceInstructionTailV1, PINNED_CONTROLLER_PROGRAM_ID,
    },
    instruction::heads as hx,
    state::heads::{
        find_heads_authority_address, find_heads_board_address, find_heads_config_address,
        HeadsAccount, HeadsBoard, HeadsConfig, HeadsMixBasis, HeadsParams,
    },
};
use common::*;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signature::{Keypair, Signer},
    transaction::Transaction,
};

/// Sends exactly `ix`, with no envelope added.
async fn send_bare(
    h: &mut Harness,
    ix: Instruction,
    signers: &[&Keypair],
) -> Result<(), solana_program_test::BanksClientError> {
    let payer = h.payer();
    let blockhash = h.ctx.get_new_latest_blockhash().await.unwrap();
    let mut all = vec![&payer];
    all.extend(
        signers
            .iter()
            .copied()
            .filter(|s| s.pubkey() != payer.pubkey()),
    );
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &all, blockhash);
    process_now(&h.ctx.banks_client, tx)
        .await
        .0
        .map_err(solana_program_test::BanksClientError::TransactionError)
}

#[tokio::test]
async fn tag_210_runs_only_under_the_governance_envelope() {
    let mut h = start().await;
    h.initialize().await.unwrap();
    let mint_authority = Keypair::new();
    let mint = h.create_mint(&mint_authority.pubkey(), 9, None).await;
    let authority = find_heads_authority_address(&program()).0;
    let ata = spl_associated_token_account::instruction::create_associated_token_account_idempotent(
        &h.ctx.payer.pubkey(),
        &authority,
        &mint,
        &spl_token::id(),
    );
    h.send(ata, &[]).await.unwrap();
    let admin = h.admin.insecure_clone();
    let payer = h.payer();
    let init = hx::init_heads_config(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        &mint,
        &spl_token::id(),
    );

    // Without the envelope the gate finds no valid tail at the payload's end, before any account
    // is read.
    assert_oracle_error(
        send_bare(&mut h, init.clone(), &[&admin]).await,
        OracleError::InvalidGovernanceTail,
    );
    // A tail without the controller's gate account is refused too (the last account is not the
    // gate).
    let mut tail_only = init.clone();
    tail_only
        .data
        .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
    assert_oracle_error(
        send_bare(&mut h, tail_only, &[&admin]).await,
        OracleError::InvalidGovernanceGatePda,
    );
    assert!(h
        .account(find_heads_config_address(&program()).0)
        .await
        .is_none());

    // With the envelope (the harness appends the tail and the gate, as senders do), the gate
    // strips them and the family runs.
    h.send(init.clone(), &[&admin]).await.unwrap();
    let config = h
        .account(find_heads_config_address(&program()).0)
        .await
        .expect("heads config created");
    let config = HeadsConfig::unpack(&config.data).unwrap();
    assert_eq!((config.amba_mint, config.amba_decimals), (mint, 9));
    // The same envelope by hand: tail appended, gate account last.
    let mut board = hx::init_board(
        &program(),
        &payer.pubkey(),
        &admin.pubkey(),
        product("RAMX"),
        HeadsParams::spec_defaults(),
        vec![hx::HeadsItemParams {
            bucket_id: [1; 32],
            basket_weight_bps: 10_000,
            mix_bps: [0, 10_000, 0],
            mix_basis: HeadsMixBasis::Assumed,
        }],
    )
    .unwrap();
    board
        .data
        .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(1).encode());
    board.accounts.push(AccountMeta::new_readonly(
        derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
        false,
    ));
    send_bare(&mut h, board, &[&admin]).await.unwrap();
    let board = h
        .account(find_heads_board_address(&program(), &product("RAMX")).0)
        .await
        .unwrap();
    assert_eq!(HeadsBoard::unpack(&board.data).unwrap().component_count, 3);
    // A stale epoch in the tail is the gate's refusal, not the family's.
    let mut stale = init;
    stale
        .data
        .extend_from_slice(&GovernanceInstructionTailV1::for_epoch(2).encode());
    stale.accounts.push(AccountMeta::new_readonly(
        derive_protocol_gate_pda(&PINNED_CONTROLLER_PROGRAM_ID, &program()).0,
        false,
    ));
    assert_oracle_error(
        send_bare(&mut h, stale, &[&admin]).await,
        OracleError::GovernanceGateEpochMismatch,
    );
}
