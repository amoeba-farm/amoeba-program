use super::*;

pub(super) fn reserve_reveal_precommitment_hash(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    auction: &Pubkey,
    auction_state: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
    params: &RevealWriterAuctionV1Params,
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(WRITER_AUCTION_RESERVE_PREIMAGE_LEN);
    bytes.extend_from_slice(WRITER_AUCTION_RESERVE_DOMAIN);
    bytes.extend_from_slice(program_id.as_ref());
    bytes.extend_from_slice(sleeve.as_ref());
    bytes.extend_from_slice(auction.as_ref());
    bytes.extend_from_slice(&auction_state.auction_nonce.to_le_bytes());
    bytes.extend_from_slice(&snapshot.policy_version.to_le_bytes());
    bytes.extend_from_slice(&snapshot.policy_hash);
    bytes.extend_from_slice(&auction_state.scenario_set_hash);
    bytes.extend_from_slice(&auction_state.risk_limit_hash);
    bytes.extend_from_slice(&snapshot.series_family_hash);
    for value in params.reserve_prices_atoms {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in params.issue_caps_atoms {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&params.nonce);
    bytes.extend_from_slice(&auction_state.bid_deadline_ts.to_le_bytes());
    bytes.extend_from_slice(&auction_state.reveal_deadline_ts.to_le_bytes());
    bytes.extend_from_slice(&auction_state.execute_deadline_ts.to_le_bytes());
    debug_assert_eq!(bytes.len(), WRITER_AUCTION_RESERVE_PREIMAGE_LEN);
    hashv(&[bytes.as_slice()]).to_bytes()
}

#[inline]
pub(super) fn slot_bound_reserve_commitment(
    precommitment: &[u8; 32],
    commit_slot: u64,
) -> [u8; 32] {
    hashv(&[
        WRITER_AUCTION_RESERVE_SLOT_DOMAIN,
        precommitment,
        &commit_slot.to_le_bytes(),
    ])
    .to_bytes()
}

#[inline]
pub(super) fn writer_auction_reveal_binding_matches(
    program_id: &Pubkey,
    sleeve: &Pubkey,
    auction: &Pubkey,
    auction_state: &WriterAuctionV1,
    snapshot: &WriterPolicySnapshotV1,
    params: &RevealWriterAuctionV1Params,
) -> bool {
    writer_auction_policy_inputs_match(auction_state, snapshot)
        && slot_bound_reserve_commitment(
            &reserve_reveal_precommitment_hash(
                program_id,
                sleeve,
                auction,
                auction_state,
                snapshot,
                params,
            ),
            auction_state.commit_slot,
        ) == auction_state.reserve_vector_commitment
}

pub(super) fn bid_index_digest(previous: &[u8; 32], record: &WriterBidIndexRecordV1) -> [u8; 32] {
    hashv(&[
        WRITER_BID_INDEX_DIGEST_DOMAIN,
        previous,
        record.bid.as_ref(),
        record.bidder.as_ref(),
        &record.order_id.to_le_bytes(),
        &[record.series_index],
        &record.bid_price_per_contract_atoms.to_le_bytes(),
        &record.requested_contract_atoms.to_le_bytes(),
        &record.escrowed_atoms.to_le_bytes(),
    ])
    .to_bytes()
}

pub(super) fn plan_digest(previous: &[u8; 32], record: &WriterBidIndexRecordV1) -> [u8; 32] {
    let status = match record.status {
        WriterBidStatus::Planned => 1,
        WriterBidStatus::Refundable => 2,
        _ => 0,
    };
    hashv(&[
        WRITER_PLAN_DIGEST_DOMAIN,
        previous,
        record.bid.as_ref(),
        &record.accepted_contract_atoms.to_le_bytes(),
        &[status],
    ])
    .to_bytes()
}

pub(super) fn bid_precedes(
    candidate: &WriterBidIndexRecordV1,
    existing: &WriterBidIndexRecordV1,
    book: &WriterSeriesBookV1,
) -> bool {
    if candidate.bid_price_per_contract_atoms != existing.bid_price_per_contract_atoms {
        return candidate.bid_price_per_contract_atoms > existing.bid_price_per_contract_atoms;
    }
    let candidate_id = book.records[usize::from(candidate.series_index)].series_id;
    let existing_id = book.records[usize::from(existing.series_index)].series_id;
    if candidate_id != existing_id {
        return candidate_id < existing_id;
    }
    if candidate.order_id != existing.order_id {
        return candidate.order_id < existing.order_id;
    }
    candidate.bid.to_bytes() < existing.bid.to_bytes()
}
