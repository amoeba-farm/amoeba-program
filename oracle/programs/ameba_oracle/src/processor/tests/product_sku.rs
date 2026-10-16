use super::*;

#[test]
fn product_sku_ids_are_complete_unique_nonzero_and_canonically_ordered() {
    let synthetic_ids = |count: u16| {
        (1..=count)
            .map(|value| [u8::try_from(value).unwrap(); 32])
            .collect::<Vec<_>>()
    };
    let ram_ids = synthetic_ids(52);
    let nand_ids = synthetic_ids(48);
    assert_eq!(validate_oracle_product_sku_ids(52, &ram_ids), Ok(52));
    assert_eq!(validate_oracle_product_sku_ids(48, &nand_ids), Ok(48));
    // Any registered product size works; there is no product table.
    assert_eq!(validate_oracle_product_sku_ids(7, &synthetic_ids(7)), Ok(7));

    let invalid = |ids: &[[u8; 32]]| {
        assert_eq!(
            validate_oracle_product_sku_ids(52, ids),
            Err(ProgramError::Custom(
                OracleError::InvalidOracleSkuCoverageManifest as u32
            ))
        );
    };
    invalid(&ram_ids[..51]);
    invalid(&synthetic_ids(60));
    let mut zero = ram_ids.clone();
    zero[0] = [0; 32];
    invalid(&zero);
    let mut duplicate = ram_ids.clone();
    duplicate[1] = duplicate[0];
    invalid(&duplicate);
    let mut descending = ram_ids.clone();
    descending.swap(0, 1);
    invalid(&descending);
    assert!(validate_oracle_product_sku_ids(0, &[]).is_err());
    assert!(validate_oracle_product_sku_ids(MAX_ORACLE_REQUIRED_SKUS + 1, &ram_ids).is_err());
}

#[test]
fn packet_safe_product_sku_chunks_finalize_only_on_the_exact_last_identifier() {
    // Synthetic monotonic values exercise the protocol shape; they are not catalog identities.
    let ids = (1..=52u16)
        .map(|value| [u8::try_from(value).unwrap(); 32])
        .collect::<Vec<_>>();
    let mut draft = synthetic_ram_product_sku_draft();
    let chunks = ids
        .chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS)
        .collect::<Vec<_>>();
    assert_eq!(
        chunks.iter().map(|chunk| chunk.len()).collect::<Vec<_>>(),
        [16, 16, 16, 4]
    );
    let mut starts = Vec::new();
    let mut finalized_root = None;
    for chunk in chunks {
        let start = draft.appended_sku_count;
        starts.push(start);
        finalized_root = append_oracle_product_sku_chunk(&mut draft, start, chunk).unwrap();
        assert_eq!(
            finalized_root.is_some(),
            usize::from(draft.appended_sku_count) == ids.len()
        );
    }
    assert_eq!(starts, [0, 16, 32, 48]);
    assert_eq!(draft.appended_sku_count, 52);
    assert_eq!(draft.last_sku_id, *ids.last().unwrap());
    assert_eq!(
        finalized_root,
        Some(oracle_sku_merkle_root(&ids).expect("vector Merkle root"))
    );
    assert_eq!(
        finalized_oracle_sku_frontier_root(&draft),
        oracle_sku_merkle_root(&ids)
    );
}

#[test]
fn streaming_product_sku_frontier_matches_vector_root_for_every_supported_count() {
    for expected_sku_count in 1..=MAX_ORACLE_REQUIRED_SKUS {
        let ids = (1..=expected_sku_count)
            .map(|value| {
                let mut sku_id = [0u8; 32];
                sku_id[..2].copy_from_slice(&value.to_be_bytes());
                sku_id
            })
            .collect::<Vec<_>>();
        let mut draft = OracleProductSkuDraft {
            is_initialized: true,
            account_discriminator: OracleProductSkuDraft::ACCOUNT_DISCRIMINATOR,
            account_version: OracleProductSkuDraft::ACCOUNT_VERSION,
            expected_sku_count,
            expected_sku_root: [5; 32],
            ..OracleProductSkuDraft::default()
        };
        for (index, sku_id) in ids.iter().enumerate() {
            let index = u16::try_from(index).unwrap();
            append_oracle_sku_frontier_leaf(
                &mut draft.merkle_frontier,
                &mut draft.frontier_mask,
                index,
                sku_id,
            )
            .unwrap();
            draft.appended_sku_count += 1;
            draft.last_sku_id = *sku_id;
            assert!(draft.has_canonical_layout());
        }
        assert_eq!(
            finalized_oracle_sku_frontier_root(&draft),
            oracle_sku_merkle_root(&ids),
            "frontier mismatch at count {expected_sku_count}"
        );
    }
}

#[test]
fn product_sku_draft_rejects_replay_gaps_overflow_and_noncanonical_chunks() {
    let invalid_error = Err(ProgramError::Custom(
        OracleError::InvalidOracleProductSkuDraft as u32,
    ));
    let mut draft = synthetic_ram_product_sku_draft();
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 1, &[[1; 32]]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &[]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &vec![[1; 32]; 17]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &[[0; 32]]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &[[1; 32], [1; 32]]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &[[2; 32], [1; 32]]),
        invalid_error
    );

    append_oracle_product_sku_chunk(&mut draft, 0, &[[1; 32]]).unwrap();
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 0, &[[2; 32]]),
        invalid_error
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut draft, 1, &[[1; 32]]),
        invalid_error
    );

    let ids = (1..=52u16)
        .map(|value| [u8::try_from(value).unwrap(); 32])
        .collect::<Vec<_>>();
    let mut nearly_full = synthetic_ram_product_sku_draft();
    for chunk in ids[..48].chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS) {
        let start = nearly_full.appended_sku_count;
        append_oracle_product_sku_chunk(&mut nearly_full, start, chunk).unwrap();
    }
    let overflow = (49u8..=53).map(|value| [value; 32]).collect::<Vec<_>>();
    assert_eq!(
        append_oracle_product_sku_chunk(&mut nearly_full, 48, &overflow),
        invalid_error
    );

    let reverted_sixty = (1u8..=60).map(|value| [value; 32]).collect::<Vec<_>>();
    let mut reverted_draft = synthetic_ram_product_sku_draft();
    for chunk in reverted_sixty[..48].chunks(MAX_ORACLE_PRODUCT_SKU_CHUNK_IDS) {
        let start = reverted_draft.appended_sku_count;
        append_oracle_product_sku_chunk(&mut reverted_draft, start, chunk).unwrap();
    }
    assert_eq!(
        append_oracle_product_sku_chunk(&mut reverted_draft, 48, &reverted_sixty[48..]),
        invalid_error
    );

    assert!(
        append_oracle_product_sku_chunk(&mut nearly_full, 48, &ids[48..])
            .unwrap()
            .is_some()
    );
    assert_eq!(
        append_oracle_product_sku_chunk(&mut nearly_full, 52, &[[53; 32]]),
        invalid_error
    );
}

#[test]
fn product_sku_draft_loader_binds_nonce_identity_layout_and_exact_allocation() {
    let program_id = Pubkey::new_unique();
    let nonce = 41u64;
    let mut draft = synthetic_ram_product_sku_draft();
    draft.draft_nonce = nonce;
    append_oracle_product_sku_chunk(&mut draft, 0, &[[1; 32], [2; 32]]).unwrap();
    let (key, bump) = derive_oracle_product_sku_draft_pda(&program_id, &draft.underlying_id, nonce);
    draft.bump = bump;
    let len = OracleProductSkuDraft::LEN;
    with_test_account_info(
        &key,
        &program_id,
        exact_test_state_data(&draft, len),
        |info| {
            assert_eq!(
                load_valid_oracle_product_sku_draft(&program_id, &draft.underlying_id, nonce, info),
                Ok(draft.clone())
            );
            assert!(load_valid_oracle_product_sku_draft(
                &program_id,
                &draft.underlying_id,
                nonce + 1,
                info
            )
            .is_err());
        },
    );

    let malformed = OracleProductSkuDraft {
        frontier_mask: 0,
        ..draft.clone()
    };
    with_test_account_info(
        &key,
        &program_id,
        exact_test_state_data(&malformed, len),
        |info| {
            assert!(load_valid_oracle_product_sku_draft(
                &program_id,
                &draft.underlying_id,
                nonce,
                info
            )
            .is_err());
        },
    );
    let mut undersized_data = exact_test_state_data(&draft, len);
    undersized_data.truncate(len - 1);
    with_test_account_info(&key, &program_id, undersized_data, |info| {
        assert!(load_valid_oracle_product_sku_draft(
            &program_id,
            &draft.underlying_id,
            nonce,
            info
        )
        .is_err());
    });
}

#[test]
fn product_manifest_loader_is_create_only_authority_for_month_root_and_count() {
    let program_id = Pubkey::new_unique();
    let mut underlying_id = [0u8; 32];
    underlying_id[..4].copy_from_slice(b"RAMX");
    let (key, bump) = derive_oracle_product_sku_manifest_pda(&program_id, &underlying_id);
    let manifest = OracleProductSkuManifest {
        is_initialized: true,
        bump,
        account_discriminator: OracleProductSkuManifest::ACCOUNT_DISCRIMINATOR,
        account_version: OracleProductSkuManifest::ACCOUNT_VERSION,
        underlying_id,
        required_sku_root: [7; 32],
        required_sku_count: 52,
        reserved: [0; 6],
        last_updated_slot: 1,
    };
    with_test_account_info(
        &key,
        &program_id,
        exact_test_state_data(&manifest, OracleProductSkuManifest::LEN),
        |info| {
            assert_eq!(
                load_valid_oracle_product_sku_manifest(&program_id, &underlying_id, info),
                Ok(manifest.clone())
            );
        },
    );

    // The registered count is authoritative for any product: an arbitrary valid tree size loads,
    // a count outside 1..=MAX_ORACLE_REQUIRED_SKUS, a zero root or a foreign identity does not.
    let other_size = OracleProductSkuManifest {
        required_sku_count: 60,
        ..manifest.clone()
    };
    with_test_account_info(
        &key,
        &program_id,
        exact_test_state_data(&other_size, OracleProductSkuManifest::LEN),
        |info| {
            assert_eq!(
                load_valid_oracle_product_sku_manifest(&program_id, &underlying_id, info),
                Ok(other_size.clone())
            );
        },
    );
    for invalid in [
        OracleProductSkuManifest {
            required_sku_count: 0,
            ..manifest.clone()
        },
        OracleProductSkuManifest {
            required_sku_count: MAX_ORACLE_REQUIRED_SKUS + 1,
            ..manifest.clone()
        },
        OracleProductSkuManifest {
            required_sku_root: [0; 32],
            ..manifest.clone()
        },
        OracleProductSkuManifest {
            underlying_id: [1; 32],
            ..manifest.clone()
        },
        OracleProductSkuManifest {
            account_version: OracleProductSkuManifest::ACCOUNT_VERSION + 1,
            ..manifest.clone()
        },
    ] {
        with_test_account_info(
            &key,
            &program_id,
            exact_test_state_data(&invalid, OracleProductSkuManifest::LEN),
            |info| {
                assert!(
                    load_valid_oracle_product_sku_manifest(&program_id, &underlying_id, info)
                        .is_err()
                );
            },
        );
    }
    with_test_account_info(&key, &system_program::id(), Vec::new(), |info| {
        assert!(load_valid_oracle_product_sku_manifest(&program_id, &underlying_id, info).is_err());
    });
}
