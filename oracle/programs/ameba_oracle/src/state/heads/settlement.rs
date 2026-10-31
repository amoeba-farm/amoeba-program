//! The canonical settlement record of a heads contract and its digest, the `Period`'s
//! `settlement_digest` (`FinalizeContract`): everything the settlement computed, in a fixed byte
//! order (little-endian integers), so anyone holding the contract account recomputes it.

use super::{HeadsContract, HEADS_CHANNELS};
use solana_program::hash::hashv;

/// Domain of [`heads_settlement_digest`].
pub const HEADS_SETTLEMENT_DIGEST_DOMAIN: &[u8] = b"ameba-oracle-heads-settlement-v1";

/// The record: the contract's identity and listing (period, board, product, expiry, window,
/// target, parameter version, base, terms commitment); per item its bucket, basket weight, mix and
/// basis, settled change and fixed-mix reference (change, flags, reason, degraded channels); per
/// component its item, channel, sentence, sources and origins at listing, origins at freeze,
/// flags, degraded reason, duplicates, eligible sources, weight, change and freeze change; then
/// the index change and the value.
pub fn heads_settlement_record(c: &HeadsContract) -> Vec<u8> {
    let items = usize::from(c.item_count).min(c.item_weight_bps.len());
    let components = usize::from(c.component_count).min(c.component_items.len());
    let mut out = Vec::with_capacity(220 + items * 64 + components * 80);
    out.extend_from_slice(c.period.as_ref());
    out.extend_from_slice(c.board.as_ref());
    out.extend_from_slice(&c.product_id);
    out.extend_from_slice(&c.expiry_ts.to_le_bytes());
    out.extend_from_slice(&c.listing_ts.to_le_bytes());
    out.extend_from_slice(&c.freeze_ts.to_le_bytes());
    out.push(c.settlement_target as u8);
    out.extend_from_slice(&c.params_version.to_le_bytes());
    out.extend_from_slice(&c.base.to_le_bytes());
    out.extend_from_slice(&c.terms_hash);
    out.push(c.item_count);
    for i in 0..items {
        out.extend_from_slice(&c.item_bucket_ids[i]);
        out.extend_from_slice(&c.item_weight_bps[i].to_le_bytes());
        for ch in 0..HEADS_CHANNELS {
            out.extend_from_slice(&c.item_mix_bps[i * HEADS_CHANNELS + ch].to_le_bytes());
        }
        out.push(c.item_mix_basis[i]);
        out.extend_from_slice(&c.item_change_ppm[i].to_le_bytes());
        out.extend_from_slice(&c.item_reference_ppm[i].to_le_bytes());
        out.push(c.item_reference_flags[i]);
        out.push(c.item_reference_reason[i]);
        out.push(c.item_degraded_channels[i]);
    }
    out.push(c.component_count);
    for k in 0..components {
        out.push(c.component_items[k]);
        out.push(c.component_channels[k]);
        out.extend_from_slice(&c.component_sentence_hashes[k]);
        out.extend_from_slice(&c.component_listed_sources[k].to_le_bytes());
        out.extend_from_slice(&c.component_listed_origins[k].to_le_bytes());
        out.extend_from_slice(&c.component_freeze_origins[k].to_le_bytes());
        out.push(c.component_flags[k]);
        out.push(c.component_degraded[k]);
        out.extend_from_slice(&c.component_duplicates[k].to_le_bytes());
        out.extend_from_slice(&c.component_eligible[k].to_le_bytes());
        out.extend_from_slice(&c.component_weight[k].to_le_bytes());
        out.extend_from_slice(&c.component_change_ppm[k].to_le_bytes());
        out.extend_from_slice(&c.component_freeze_change_ppm[k].to_le_bytes());
    }
    out.extend_from_slice(&c.index_change_ppm.to_le_bytes());
    out.extend_from_slice(&c.value.to_le_bytes());
    out
}

/// `sha256(HEADS_SETTLEMENT_DIGEST_DOMAIN ‖ heads_settlement_record(contract))`.
pub fn heads_settlement_digest(contract: &HeadsContract) -> [u8; 32] {
    hashv(&[
        HEADS_SETTLEMENT_DIGEST_DOMAIN,
        &heads_settlement_record(contract),
    ])
    .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_covers_the_record_and_nothing_else() {
        let mut contract = HeadsContract {
            item_count: 1,
            component_count: 3,
            value: 7,
            ..HeadsContract::default()
        };
        let digest = heads_settlement_digest(&contract);
        contract.settled_ts = 99;
        contract.updated_ts = 99;
        assert_eq!(heads_settlement_digest(&contract), digest);
        contract.component_change_ppm[2] = 1;
        assert_ne!(heads_settlement_digest(&contract), digest);
        contract.component_change_ppm[2] = 0;
        // Unused components beyond the count are not part of it.
        contract.component_change_ppm[3] = 1;
        assert_eq!(heads_settlement_digest(&contract), digest);
        // 166 bytes of listing, 60 per item, 1 + 70 per component, 16 of result.
        assert_eq!(
            heads_settlement_record(&contract).len(),
            166 + 60 + 1 + 3 * 70 + 16
        );
    }
}
