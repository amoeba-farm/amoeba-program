use super::*;
use crate::state::heads::{
    HeadsChannel, HeadsMixBasis, HeadsSettlementTarget, HEADS_MAX_COMPONENTS, HEADS_MAX_ITEMS,
};

fn listed() -> HeadsListedMix {
    HeadsListedMix {
        mix_bps: [1_000, 4_600, 4_400],
        mix_basis: HeadsMixBasis::Assumed,
    }
}

fn item(weight: u16) -> HeadsItemParams {
    HeadsItemParams {
        bucket_id: [weight as u8; 32],
        basket_weight_bps: weight,
        mix_bps: [2_000, 5_000, 3_000],
        mix_basis: HeadsMixBasis::Assumed,
    }
}

fn reveal() -> RevealClaimParams {
    RevealClaimParams {
        value: 101_000_000,
        source_ts: 1_761_955_200,
        origin_id: [5; 32],
        lta: HeadsLtaTerms {
            effective_from_ts: 1_759_276_800,
            next_reset_ts: 1_767_225_600,
            floor: 90_000_000,
            ceiling: 0,
        },
        salt: [6; 32],
        evidence_kind: 4,
        evidence_commitment: [7; 32],
    }
}

/// One instruction per selector, with the payload length each must have (`None`: variable).
pub(crate) fn samples() -> Vec<(HeadsInstruction, Option<usize>)> {
    use HeadsInstruction as I;
    let name = "Mouser DDR5-4800 16Gb spot".to_owned();
    let name_len = name.len();
    vec![
        (
            I::InitHeadsConfig(InitHeadsConfigParams {
                amba_mint: Pubkey::new_from_array([1; 32]),
                amba_token_program: Pubkey::new_from_array([2; 32]),
            }),
            Some(64),
        ),
        (
            I::InitBoard(InitBoardParams {
                product_id: [3; 32],
                params: HeadsParams::spec_defaults(),
                items: vec![item(6_000), item(4_000)],
            }),
            Some(32 + 120 + 4 + 2 * 41),
        ),
        (
            I::SetBoardParams(SetBoardParamsParams {
                expected_params_version: 3,
                params: HeadsParams::spec_defaults(),
            }),
            Some(124),
        ),
        (I::FundBoard { amount: 8 }, Some(8)),
        (I::Stake { amount: 9 }, Some(8)),
        (I::Unstake { amount: 10 }, Some(8)),
        (
            I::PostHead(PostHeadParams {
                expected_head_id: 11,
                component: 23,
                access_class: HeadAccessClass::LoginZkTls,
                amount: 12,
                spec_hash: [13; 32],
                name,
            }),
            Some(4 + 1 + 1 + 8 + 32 + 4 + name_len),
        ),
        (I::Delegate { amount: 14 }, Some(8)),
        (I::Undelegate { amount: 15 }, Some(8)),
        (I::SyncDelegation, Some(0)),
        (I::ExpireHead, Some(0)),
        (
            I::CommitClaim(CommitClaimParams {
                expected_seq: 17,
                kind: HeadClaimKind::Update,
                commit_hash: [18; 32],
                bond: 19,
            }),
            Some(45),
        ),
        (I::RevealClaim(reveal()), Some(145)),
        (
            I::ChallengeClaim(ChallengeClaimParams {
                bond: 20,
                reason: HeadChallengeReason::ExpiredObservation,
                alternative_value: 21,
                alternative_source_ts: 22,
                evidence_kind: 3,
                evidence_commitment: [23; 32],
                reason_hash: [24; 32],
            }),
            Some(90),
        ),
        (
            I::RuleChallenge(RuleChallengeParams {
                kind: HeadChallengeKind::Source,
                ruling: HeadChallengeRuling::Lost,
                expected_epoch: 25,
            }),
            Some(10),
        ),
        (I::FinalizeClaim, Some(0)),
        (I::SettleChallenge, Some(0)),
        (I::RefundExpiredCommit, Some(0)),
        (
            I::ChallengeSource(ChallengeSourceParams {
                bond: 26,
                reason: HeadChallengeReason::DuplicateOrigin,
                evidence_kind: 0,
                evidence_commitment: [0; 32],
                reason_hash: [27; 32],
            }),
            Some(74),
        ),
        (I::FinalizeSourceChallenges, Some(0)),
        (
            I::ListContract(ListContractParams {
                base: 100_000_000,
                settlement_target: HeadsSettlementTarget::FixedMixReference,
                terms_hash: [28; 32],
                items: vec![listed(), listed()],
            }),
            Some(41 + 4 + 2 * 7),
        ),
        (
            I::SettleComponent(SettleComponentParams {
                component: 23,
                first_source: 30,
            }),
            Some(3),
        ),
        (I::FinalizeContract, Some(0)),
        (I::ClaimRewards, Some(0)),
        (I::ClaimPrize, Some(0)),
        (I::BurnPending, Some(0)),
        (
            I::SetItem(SetItemParams {
                index: 7,
                item: HeadsItemParams {
                    bucket_id: [31; 32],
                    basket_weight_bps: 0,
                    mix_bps: [0, 10_000, 0],
                    mix_basis: HeadsMixBasis::Matched,
                },
            }),
            Some(42),
        ),
        (
            I::SetComponent(SetComponentParams {
                index: 23,
                item: 7,
                channel: HeadsChannel::Lta,
                sentence_hash: [32; 32],
            }),
            Some(35),
        ),
        (
            I::CreateQuarterPeriod(CreateQuarterPeriodParams {
                product_id: [33; 32],
                planned_listing_ts: 1_798_761_600,
                // 2027-04-01 00:00 UTC.
                expiry_ts: 1_806_537_600,
            }),
            Some(48),
        ),
        (I::RollOverPrizes, Some(0)),
        (I::MigrateHead { contracts: 2 }, Some(1)),
        (I::SetRulingDeadline { secs: 86_400 }, Some(8)),
    ]
}

fn encode(instruction: &HeadsInstruction) -> Vec<u8> {
    instruction.data().unwrap()
}

fn decode(data: &[u8]) -> Result<HeadsInstruction, crate::ProgramError> {
    assert_eq!(data.first(), Some(&ORACLE_HEADS_V1_TAG));
    HeadsInstruction::unpack(&data[1..])
}

#[test]
fn every_selector_roundtrips_at_its_exact_length() {
    let samples = samples();
    assert_eq!(samples.len(), usize::from(selector::COUNT));
    for (index, (instruction, payload_len)) in samples.iter().enumerate() {
        assert_eq!(
            usize::from(instruction.selector()),
            index,
            "{instruction:?}"
        );
        let data = encode(instruction);
        assert_eq!(data[0], 210);
        assert_eq!(data[1], instruction.selector());
        if let Some(len) = payload_len {
            assert_eq!(data.len(), 2 + len, "selector {index}");
        }
        assert_eq!(&decode(&data).unwrap(), instruction);
        // Truncated and trailing bytes reject.
        assert!(decode(&data[..data.len() - 1]).is_err(), "selector {index}");
        let mut trailing = data.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err(), "selector {index}");
    }
}

#[test]
fn the_tag_and_selectors_are_pinned() {
    assert_eq!(ORACLE_HEADS_V1_TAG, 210);
    use selector::*;
    assert_eq!(
        [
            INIT_HEADS_CONFIG,
            INIT_BOARD,
            SET_BOARD_PARAMS,
            FUND_BOARD,
            STAKE,
            UNSTAKE,
            POST_HEAD,
            DELEGATE,
            UNDELEGATE,
            SYNC_DELEGATION,
            EXPIRE_HEAD,
            COMMIT_CLAIM,
            REVEAL_CLAIM,
            CHALLENGE_CLAIM,
            RULE_CHALLENGE,
            FINALIZE_CLAIM,
            SETTLE_CHALLENGE,
            REFUND_EXPIRED_COMMIT,
            CHALLENGE_SOURCE,
            FINALIZE_SOURCE_CHALLENGES,
            LIST_CONTRACT,
            SETTLE_COMPONENT,
            FINALIZE_CONTRACT,
            CLAIM_REWARDS,
            CLAIM_PRIZE,
            BURN_PENDING,
            SET_ITEM,
            SET_COMPONENT,
            CREATE_QUARTER_PERIOD,
            ROLL_OVER_PRIZES,
            MIGRATE_HEAD,
            SET_RULING_DEADLINE,
        ],
        core::array::from_fn::<u8, 32, _>(|i| i as u8)
    );
}

#[test]
fn unassigned_selectors_and_empty_payloads_reject() {
    assert!(HeadsInstruction::unpack(&[]).is_err());
    for selector in selector::COUNT..=u8::MAX {
        assert!(HeadsInstruction::unpack(&[selector]).is_err(), "{selector}");
        assert!(HeadsInstruction::unpack(&[selector, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}

/// Encodes `instruction` with the serializer's checks bypassed by patching `data` at `offset`.
fn patched(instruction: HeadsInstruction, offset: usize, bytes: &[u8]) -> Vec<u8> {
    let mut data = encode(&instruction);
    data[2 + offset..2 + offset + bytes.len()].copy_from_slice(bytes);
    data
}

#[test]
fn out_of_domain_values_reject_in_the_decoder() {
    use HeadsInstruction as I;
    let samples = samples();
    let get = |selector: u8| samples[usize::from(selector)].0.clone();
    let rejects = |data: Vec<u8>| assert!(decode(&data).is_err(), "{data:?}");

    // Items: a mix that does not add up to 10,000; an unknown basis; no items; too many.
    let board = get(selector::INIT_BOARD);
    rejects(patched(
        board.clone(),
        32 + 120 + 4 + 34,
        &4_999u16.to_le_bytes(),
    ));
    rejects(patched(board.clone(), 32 + 120 + 4 + 40, &[3]));
    rejects(patched(board.clone(), 32 + 120, &0u32.to_le_bytes()));
    if let I::InitBoard(mut p) = board {
        p.items = vec![item(1_250); HEADS_MAX_ITEMS + 1];
        assert!(I::InitBoard(p.clone()).data().is_err());
        let mut data = vec![210, selector::INIT_BOARD];
        p.serialize(&mut data).unwrap();
        rejects(data);
    }
    // A component or item index out of range, a zero sentence, an unknown channel.
    let component = get(selector::SET_COMPONENT);
    rejects(patched(component.clone(), 0, &[HEADS_MAX_COMPONENTS as u8]));
    rejects(patched(component.clone(), 1, &[HEADS_MAX_ITEMS as u8]));
    rejects(patched(component.clone(), 2, &[3]));
    rejects(patched(component.clone(), 3, &[0; 32]));
    // The index is the mold's item * 3 + channel.
    rejects(patched(component, 0, &[22]));
    rejects(patched(
        get(selector::SET_ITEM),
        0,
        &[HEADS_MAX_ITEMS as u8],
    ));
    rejects(patched(
        get(selector::POST_HEAD),
        4,
        &[HEADS_MAX_COMPONENTS as u8],
    ));
    rejects(patched(get(selector::POST_HEAD), 5, &[4]));
    // A name over the limit, or not UTF-8.
    if let I::PostHead(mut p) = get(selector::POST_HEAD) {
        p.name = "x".repeat(HEAD_NAME_MAX_BYTES + 1);
        assert!(I::PostHead(p.clone()).data().is_err());
        let mut data = vec![210, selector::POST_HEAD];
        BorshSerialize::serialize(&p, &mut data).unwrap();
        rejects(data);
        p.name = "x".repeat(HEAD_NAME_MAX_BYTES);
        assert!(decode(&I::PostHead(p).data().unwrap()).is_ok());
    }
    rejects(patched(get(selector::POST_HEAD), 50, &[0xff]));
    // Reveal: a zero origin, LTA terms out of order, evidence kind and commitment disagreeing.
    rejects(patched(get(selector::REVEAL_CLAIM), 16, &[0; 32]));
    rejects(patched(
        get(selector::REVEAL_CLAIM),
        48,
        &u64::MAX.to_le_bytes(),
    ));
    let mut floor_over_ceiling = encode(&I::RevealClaim(RevealClaimParams {
        lta: HeadsLtaTerms {
            floor: 2,
            ceiling: 3,
            ..HeadsLtaTerms::default()
        },
        ..reveal()
    }));
    floor_over_ceiling[2 + 64..2 + 72].copy_from_slice(&4u64.to_le_bytes());
    rejects(floor_over_ceiling);
    for kind in [1u8, 5] {
        rejects(patched(get(selector::REVEAL_CLAIM), 112, &[kind]));
    }
    rejects(patched(get(selector::REVEAL_CLAIM), 113, &[0; 32]));
    // Challenges: an unknown reason, an alternative value without its time, bad evidence.
    rejects(patched(get(selector::CHALLENGE_CLAIM), 8, &[4]));
    rejects(patched(
        get(selector::CHALLENGE_CLAIM),
        17,
        &0u64.to_le_bytes(),
    ));
    rejects(patched(get(selector::CHALLENGE_CLAIM), 25, &[0]));
    rejects(patched(get(selector::CHALLENGE_SOURCE), 8, &[4]));
    rejects(patched(get(selector::CHALLENGE_SOURCE), 10, &[1; 32]));
    // A ruling must decide, under a real epoch.
    rejects(patched(get(selector::RULE_CHALLENGE), 1, &[0]));
    rejects(patched(
        get(selector::RULE_CHALLENGE),
        2,
        &0u64.to_le_bytes(),
    ));
    rejects(patched(get(selector::RULE_CHALLENGE), 0, &[2]));
    // Listing: an unknown target, a zero terms commitment.
    rejects(patched(get(selector::LIST_CONTRACT), 8, &[2]));
    rejects(patched(get(selector::LIST_CONTRACT), 9, &[0; 32]));
    // A listing with no items, an invalid mix; a quarter period at no quarter boundary, or
    // listing at its expiry.
    let list = get(selector::LIST_CONTRACT);
    rejects(patched(list.clone(), 41, &0u32.to_le_bytes()));
    rejects(patched(list, 45, &[0, 0]));
    let quarter = get(selector::CREATE_QUARTER_PERIOD);
    rejects(patched(
        quarter.clone(),
        40,
        &1_801_440_000u64.to_le_bytes(),
    ));
    rejects(patched(quarter, 32, &1_806_537_600u64.to_le_bytes()));
    // A component out of range, an unknown claim kind.
    rejects(patched(
        get(selector::SETTLE_COMPONENT),
        0,
        &[HEADS_MAX_COMPONENTS as u8],
    ));
    rejects(patched(get(selector::COMMIT_CLAIM), 4, &[2]));
}

#[test]
fn the_serializer_refuses_what_the_decoder_refuses() {
    use HeadsInstruction as I;
    assert!(I::RuleChallenge(RuleChallengeParams {
        kind: HeadChallengeKind::Claim,
        ruling: HeadChallengeRuling::Pending,
        expected_epoch: 1,
    })
    .data()
    .is_err());
    assert!(I::RevealClaim(RevealClaimParams {
        origin_id: [0; 32],
        ..reveal()
    })
    .data()
    .is_err());
    assert!(I::ListContract(ListContractParams {
        base: 1,
        settlement_target: HeadsSettlementTarget::ContractChannel,
        terms_hash: [0; 32],
        items: vec![listed()],
    })
    .data()
    .is_err());
    assert!(I::ListContract(ListContractParams {
        base: 1,
        settlement_target: HeadsSettlementTarget::ContractChannel,
        terms_hash: [1; 32],
        items: vec![],
    })
    .data()
    .is_err());
    assert!(I::CreateQuarterPeriod(CreateQuarterPeriodParams {
        product_id: [1; 32],
        planned_listing_ts: 0,
        expiry_ts: 1_801_440_000,
    })
    .data()
    .is_err());
    assert!(I::SetItem(SetItemParams {
        index: 0,
        item: HeadsItemParams {
            mix_bps: [1, 1, 1],
            ..item(1)
        },
    })
    .data()
    .is_err());
    assert!(I::SettleComponent(SettleComponentParams {
        component: HEADS_MAX_COMPONENTS as u8,
        first_source: 0,
    })
    .data()
    .is_err());
}

/// The program's cursor decoder accepts exactly what Borsh's reader accepts, with the same
/// value: every sample, each of its truncations, and each with a byte too many.
#[test]
fn the_cursor_decoder_agrees_with_borsh() {
    for (instruction, _) in samples() {
        let data = encode(&instruction);
        let payload = &data[1..];
        let mut cases: Vec<Vec<u8>> = (0..payload.len()).map(|n| payload[..n].to_vec()).collect();
        cases.push(payload.to_vec());
        cases.push([payload, &[0]].concat());
        for case in cases {
            let borsh = HeadsInstruction::try_from_slice(&case).ok();
            let cursor = HeadsInstruction::unpack(&case).ok();
            assert_eq!(cursor, borsh, "{case:?}");
        }
        assert_eq!(HeadsInstruction::unpack(payload).ok(), Some(instruction));
    }
}

/// The hand-written commitment payload is Borsh's encoding of the reveal: commitments made by any
/// client before (Borsh) stay valid.
#[test]
fn the_claim_commitment_is_over_the_borsh_encoding() {
    let (head, fetcher) = (
        Pubkey::new_from_array([1; 32]),
        Pubkey::new_from_array([2; 32]),
    );
    let r = reveal();
    let expected = hashv(&[
        HEADS_CLAIM_COMMITMENT_DOMAIN,
        head.as_ref(),
        &3u32.to_le_bytes(),
        fetcher.as_ref(),
        &borsh::to_vec(&r).unwrap(),
    ])
    .to_bytes();
    assert_eq!(heads_claim_commitment(&head, 3, &fetcher, &r), expected);
}

#[test]
fn the_claim_commitment_binds_every_revealed_field() {
    let head = Pubkey::new_from_array([1; 32]);
    let fetcher = Pubkey::new_from_array([2; 32]);
    let base = heads_claim_commitment(&head, 3, &fetcher, &reveal());
    assert_eq!(base, heads_claim_commitment(&head, 3, &fetcher, &reveal()));
    let mut payload = Vec::new();
    reveal().serialize(&mut payload).unwrap();
    assert_eq!(
        base,
        hashv(&[
            HEADS_CLAIM_COMMITMENT_DOMAIN,
            head.as_ref(),
            &3u32.to_le_bytes(),
            fetcher.as_ref(),
            &payload,
        ])
        .to_bytes()
    );
    assert_ne!(base, heads_claim_commitment(&head, 4, &fetcher, &reveal()));
    assert_ne!(base, heads_claim_commitment(&fetcher, 3, &head, &reveal()));
    let variants = [
        RevealClaimParams {
            value: 1,
            ..reveal()
        },
        RevealClaimParams {
            source_ts: 1,
            ..reveal()
        },
        RevealClaimParams {
            origin_id: [9; 32],
            ..reveal()
        },
        RevealClaimParams {
            lta: HeadsLtaTerms {
                ceiling: 1,
                floor: 0,
                ..reveal().lta
            },
            ..reveal()
        },
        RevealClaimParams {
            salt: [9; 32],
            ..reveal()
        },
        RevealClaimParams {
            evidence_kind: 3,
            ..reveal()
        },
        RevealClaimParams {
            evidence_commitment: [9; 32],
            ..reveal()
        },
    ];
    for variant in variants {
        assert_ne!(base, heads_claim_commitment(&head, 3, &fetcher, &variant));
    }
    assert_ne!(heads_reason_hash(b"a"), heads_reason_hash(b"b"));
}

mod builders {
    use super::*;
    use crate::state::heads::*;
    use crate::state::{derive_user_collateral_pda, find_config_address};
    use solana_program::instruction::{AccountMeta, Instruction};

    const PRODUCT: [u8; 32] = *b"RAMX\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

    fn program() -> Pubkey {
        Pubkey::new_from_array([200; 32])
    }

    fn k(byte: u8) -> Pubkey {
        Pubkey::new_from_array([byte; 32])
    }

    fn check(ix: &Instruction, expected: HeadsInstruction, accounts: usize, signers: &[usize]) {
        assert_eq!(ix.program_id, program());
        assert_eq!(decode(&ix.data).unwrap(), expected);
        assert_eq!(ix.accounts.len(), accounts, "{expected:?}");
        let signed: Vec<usize> = ix
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| a.is_signer)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(signed, signers, "{expected:?}");
    }

    fn at(ix: &Instruction, index: usize) -> &AccountMeta {
        &ix.accounts[index]
    }

    #[test]
    fn every_builder_encodes_its_selector_with_its_account_list() {
        let p = program();
        let a = HeadAddresses::derive(&p, &PRODUCT, 4);
        let c = ContractAddresses::derive(&p, &PRODUCT, 1_798_761_600);
        let mint = k(1);
        let t22 = crate::token_instruction::TOKEN_2022_PROGRAM_ID;
        let (owner, fetcher, challenger, payer) = (k(2), k(3), k(4), k(5));
        let seats = [k(6), k(7), k(8)];
        let council = k(9);

        let ix = init_heads_config(&p, &payer, &k(10), &mint, &t22);
        check(
            &ix,
            HeadsInstruction::InitHeadsConfig(InitHeadsConfigParams {
                amba_mint: mint,
                amba_token_program: t22,
            }),
            9,
            &[0, 1],
        );
        assert_eq!(at(&ix, 3).pubkey, find_heads_config_address(&p).0);
        assert_eq!(
            at(&ix, 6).pubkey,
            find_heads_amba_vault_address(&p, &mint, &t22)
        );

        let ix = init_board(
            &p,
            &payer,
            &k(10),
            PRODUCT,
            HeadsParams::spec_defaults(),
            vec![item(10_000)],
        )
        .unwrap();
        check(
            &ix,
            HeadsInstruction::InitBoard(InitBoardParams {
                product_id: PRODUCT,
                params: HeadsParams::spec_defaults(),
                items: vec![item(10_000)],
            }),
            6,
            &[0, 1],
        );
        assert_eq!(at(&ix, 4).pubkey, a.board);
        assert!(init_board(
            &p,
            &payer,
            &k(10),
            PRODUCT,
            HeadsParams::spec_defaults(),
            vec![]
        )
        .is_err());

        let ix = set_board_params(&p, &k(10), &PRODUCT, 2, HeadsParams::spec_defaults());
        check(
            &ix,
            HeadsInstruction::SetBoardParams(SetBoardParamsParams {
                expected_params_version: 2,
                params: HeadsParams::spec_defaults(),
            }),
            3,
            &[0],
        );
        let ix = set_item(&p, &k(10), &PRODUCT, 1, item(5_000)).unwrap();
        check(
            &ix,
            HeadsInstruction::SetItem(SetItemParams {
                index: 1,
                item: item(5_000),
            }),
            3,
            &[0],
        );
        let ix = set_component(
            &p,
            &k(10),
            &PRODUCT,
            2,
            0,
            HeadsChannel::Lta,
            [3; 32],
            &k(11),
        )
        .unwrap();
        check(
            &ix,
            HeadsInstruction::SetComponent(SetComponentParams {
                index: 2,
                item: 0,
                channel: HeadsChannel::Lta,
                sentence_hash: [3; 32],
            }),
            4,
            &[0],
        );
        assert_eq!(at(&ix, 3).pubkey, k(11));

        // Every creating builder: the sponsor pays (account 0), the owner signs (account 1).
        let ix = fund_board(&p, &payer, &owner, &PRODUCT, 24_321, 5);
        check(&ix, HeadsInstruction::FundBoard { amount: 5 }, 7, &[0, 1]);
        assert_eq!((at(&ix, 0).pubkey, at(&ix, 1).pubkey), (payer, owner));
        assert!(at(&ix, 0).is_writable && !at(&ix, 1).is_writable);
        assert_eq!(at(&ix, 3).pubkey, derive_user_collateral_pda(&p, &owner).0);
        assert_eq!(
            at(&ix, 5).pubkey,
            find_heads_epoch_address(&p, &a.board, 24_321).0
        );

        let ix = stake(&p, &payer, &owner, &k(12), &mint, &t22, 6);
        check(&ix, HeadsInstruction::Stake { amount: 6 }, 9, &[0, 1]);
        assert_eq!(at(&ix, 3).pubkey, find_heads_stake_address(&p, &owner).0);
        let ix = unstake(&p, &owner, &k(12), &mint, &t22, 7);
        check(&ix, HeadsInstruction::Unstake { amount: 7 }, 8, &[0]);
        assert_eq!(at(&ix, 3).pubkey, find_heads_authority_address(&p).0);

        let ix = post_head(
            &p,
            &payer,
            &owner,
            &PRODUCT,
            4,
            1,
            HeadAccessClass::PublicPage,
            8,
            "SK hynix",
            [9; 32],
            &k(13),
        )
        .unwrap();
        check(
            &ix,
            HeadsInstruction::PostHead(PostHeadParams {
                expected_head_id: 4,
                component: 1,
                access_class: HeadAccessClass::PublicPage,
                amount: 8,
                spec_hash: [9; 32],
                name: "SK hynix".into(),
            }),
            10,
            &[0, 1],
        );
        assert_eq!(at(&ix, 5).pubkey, a.head);
        assert_eq!(at(&ix, 6).pubkey, a.delegation(&p, &owner));
        assert_eq!(at(&ix, 9).pubkey, derive_user_collateral_pda(&p, &owner).0);
        assert!(post_head(
            &p,
            &owner,
            &owner,
            &PRODUCT,
            4,
            1,
            HeadAccessClass::PublicPage,
            8,
            &"x".repeat(65),
            [9; 32],
            &k(13)
        )
        .is_err());

        check(
            &delegate(&p, &payer, &owner, &PRODUCT, 4, 9),
            HeadsInstruction::Delegate { amount: 9 },
            8,
            &[0, 1],
        );
        check(
            &undelegate(&p, &owner, &PRODUCT, 4, 9),
            HeadsInstruction::Undelegate { amount: 9 },
            6,
            &[0],
        );
        // A closing builder sends the rent to the recorded payer.
        let ix = sync_delegation(&p, &PRODUCT, 4, &owner, &payer);
        check(&ix, HeadsInstruction::SyncDelegation, 6, &[]);
        assert!(at(&ix, 5).is_writable && at(&ix, 5).pubkey == payer);
        assert_eq!(at(&ix, 4).pubkey, a.delegation(&p, &owner));
        check(
            &expire_head(&p, &PRODUCT, 4),
            HeadsInstruction::ExpireHead,
            2,
            &[],
        );

        let commitment = heads_claim_commitment(&a.head, 3, &fetcher, &reveal());
        let ix = commit_claim(
            &p,
            &payer,
            &fetcher,
            &PRODUCT,
            4,
            3,
            HeadClaimKind::Fill,
            commitment,
            11,
        );
        check(
            &ix,
            HeadsInstruction::CommitClaim(CommitClaimParams {
                expected_seq: 3,
                kind: HeadClaimKind::Fill,
                commit_hash: commitment,
                bond: 11,
            }),
            8,
            &[0, 1],
        );
        assert_eq!(at(&ix, 5).pubkey, a.claim(&p, 3));
        assert_eq!(
            at(&ix, 6).pubkey,
            crate::state::derive_user_collateral_pda(&p, &fetcher).0
        );
        let ix = reveal_claim(&p, &fetcher, &PRODUCT, 4, 3, reveal(), Some(&k(14))).unwrap();
        check(&ix, HeadsInstruction::RevealClaim(reveal()), 5, &[0]);
        assert_eq!(at(&ix, 4).pubkey, k(14));
        let none = RevealClaimParams {
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            ..reveal()
        };
        let ix = reveal_claim(&p, &fetcher, &PRODUCT, 4, 3, none, None).unwrap();
        assert_eq!(at(&ix, 4).pubkey, solana_sdk_ids::system_program::ID);

        let challenge = ChallengeClaimParams {
            bond: 12,
            reason: HeadChallengeReason::OffChannel,
            alternative_value: 0,
            alternative_source_ts: 0,
            evidence_kind: 0,
            evidence_commitment: [0; 32],
            reason_hash: heads_reason_hash(b"spot lot, not a contract price"),
        };
        let ix = challenge_claim(
            &p,
            &payer,
            &challenger,
            &PRODUCT,
            4,
            3,
            challenge.clone(),
            None,
        )
        .unwrap();
        check(
            &ix,
            HeadsInstruction::ChallengeClaim(challenge),
            10,
            &[0, 1],
        );
        let claim_challenge = find_head_challenge_address(&p, &a.claim(&p, 3), &challenger).0;
        assert_eq!(at(&ix, 6).pubkey, claim_challenge);

        let ix = rule_claim_challenge(&p, &council, &seats, &PRODUCT, 4, 3, &challenger, true, 2);
        check(
            &ix,
            HeadsInstruction::RuleChallenge(RuleChallengeParams {
                kind: HeadChallengeKind::Claim,
                ruling: HeadChallengeRuling::Won,
                expected_epoch: 2,
            }),
            8,
            &[1, 2, 3],
        );
        assert_eq!(at(&ix, 5).pubkey, claim_challenge);
        assert_eq!(at(&ix, 7).pubkey, a.board);
        let ix = rule_source_challenge(&p, &council, &seats, &PRODUCT, 4, &challenger, false, 2);
        check(
            &ix,
            HeadsInstruction::RuleChallenge(RuleChallengeParams {
                kind: HeadChallengeKind::Source,
                ruling: HeadChallengeRuling::Lost,
                expected_epoch: 2,
            }),
            8,
            &[1, 2, 3],
        );
        assert_eq!(
            at(&ix, 5).pubkey,
            find_head_challenge_address(&p, &a.head, &challenger).0
        );

        let ix = finalize_claim(&p, &payer, &PRODUCT, 4, 3, &fetcher, 24_321);
        check(&ix, HeadsInstruction::FinalizeClaim, 8, &[0]);
        assert_eq!(
            at(&ix, 5).pubkey,
            crate::state::derive_user_collateral_pda(&p, &fetcher).0
        );
        let ix = settle_claim_challenge(&p, &PRODUCT, 4, 3, &challenger, &payer);
        check(&ix, HeadsInstruction::SettleChallenge, 8, &[]);
        assert_eq!(at(&ix, 7).pubkey, payer);
        assert_eq!(at(&ix, 4).pubkey, a.claim(&p, 3));
        let ix = settle_source_challenge(&p, &PRODUCT, 4, &challenger, &challenger);
        check(&ix, HeadsInstruction::SettleChallenge, 8, &[]);
        assert_eq!(at(&ix, 4).pubkey, solana_sdk_ids::system_program::ID);
        check(
            &refund_expired_commit(&p, &PRODUCT, 4, 3, &fetcher),
            HeadsInstruction::RefundExpiredCommit,
            5,
            &[],
        );
        assert_eq!(
            at(&refund_expired_commit(&p, &PRODUCT, 4, 3, &fetcher), 1).pubkey,
            a.board
        );

        let source = ChallengeSourceParams {
            bond: 13,
            reason: HeadChallengeReason::DuplicateOrigin,
            evidence_kind: 2,
            evidence_commitment: [15; 32],
            reason_hash: [16; 32],
        };
        let ix = challenge_source(
            &p,
            &payer,
            &challenger,
            &PRODUCT,
            4,
            source.clone(),
            Some(&k(17)),
        )
        .unwrap();
        check(&ix, HeadsInstruction::ChallengeSource(source), 10, &[0, 1]);
        assert_eq!(
            at(&ix, 9).pubkey,
            find_heads_stake_address(&p, &challenger).0
        );
        check(
            &finalize_source_challenges(&p, &PRODUCT, 4, &owner),
            HeadsInstruction::FinalizeSourceChallenges,
            6,
            &[],
        );

        let ix = list_contract(
            &p,
            &payer,
            &k(18),
            &PRODUCT,
            1_798_761_600,
            100_000_000,
            HeadsSettlementTarget::ContractChannel,
            &[listed()],
            [19; 32],
            &k(20),
        )
        .unwrap();
        check(
            &ix,
            HeadsInstruction::ListContract(ListContractParams {
                base: 100_000_000,
                settlement_target: HeadsSettlementTarget::ContractChannel,
                terms_hash: [19; 32],
                items: vec![listed()],
            }),
            9,
            &[0, 2],
        );
        assert!(list_contract(
            &p,
            &payer,
            &k(18),
            &PRODUCT,
            1_798_761_600,
            1,
            HeadsSettlementTarget::ContractChannel,
            &[],
            [19; 32],
            &k(20),
        )
        .is_err());
        assert_eq!(at(&ix, 3).pubkey, c.period);
        assert_eq!(at(&ix, 6).pubkey, c.contract);
        assert_eq!(at(&ix, 7).pubkey, k(20));
        let ix = create_quarter_period(&p, &payer, &k(18), &PRODUCT, 0, 1_806_537_600).unwrap();
        check(
            &ix,
            HeadsInstruction::CreateQuarterPeriod(CreateQuarterPeriodParams {
                product_id: PRODUCT,
                planned_listing_ts: 0,
                expiry_ts: 1_806_537_600,
            }),
            6,
            &[0, 2],
        );
        assert_eq!(
            at(&ix, 4).pubkey,
            ContractAddresses::derive(&p, &PRODUCT, 1_806_537_600).period
        );
        assert!(create_quarter_period(&p, &payer, &k(18), &PRODUCT, 0, 1_801_440_000).is_err());

        let ix = settle_component(&p, &payer, &PRODUCT, 1_798_761_600, 2, 1, &[4, 9]).unwrap();
        check(
            &ix,
            HeadsInstruction::SettleComponent(SettleComponentParams {
                component: 2,
                first_source: 1,
            }),
            7,
            &[0],
        );
        assert_eq!(at(&ix, 3).pubkey, c.weights(&p, 2));
        assert_eq!(at(&ix, 5).pubkey, a.head);
        assert_eq!(at(&ix, 6).pubkey, find_head_address(&p, &a.board, 9).0);
        check(
            &finalize_contract(&p, &PRODUCT, 1_798_761_600),
            HeadsInstruction::FinalizeContract,
            3,
            &[],
        );
        let ix = claim_rewards(&p, &owner, &PRODUCT, 4);
        check(&ix, HeadsInstruction::ClaimRewards, 6, &[0]);
        assert_eq!(at(&ix, 1).pubkey, find_config_address(&p).0);
        check(
            &claim_prize(&p, &fetcher, &PRODUCT, 4, 3, 24_321),
            HeadsInstruction::ClaimPrize,
            7,
            &[0],
        );
        let ix = burn_pending(&p, &PRODUCT, &mint, &t22, &k(21));
        check(&ix, HeadsInstruction::BurnPending, 10, &[]);
        assert_eq!(at(&ix, 9).pubkey, crate::token_instruction::id());
    }
}
