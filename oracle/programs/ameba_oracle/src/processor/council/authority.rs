use super::*;
use crate::compact_error::CompactAccountInfo;

#[derive(Debug)]
pub struct CouncilView {
    pub epoch: u64,
    pub seats: [Pubkey; 5],
    pub digest: [u8; 32],
}

/// Exact governance-controller V3 Config wire adapter. The roster is never editable here.
#[inline(never)]
pub fn decode_council(
    controller: &Pubkey,
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<CouncilView, ProgramError> {
    decode_council_v3_with_profile(
        controller,
        key,
        owner,
        data,
        cfg!(feature = "localnet-governance"),
    )
}

/// Read-only adapter for clients that choose a profile from their trusted deployment.
/// On-chain authorization always calls `decode_council`, which fixes the compiled profile.
#[inline(never)]
pub fn decode_council_v3_with_profile(
    controller: &Pubkey,
    key: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
    development: bool,
) -> Result<CouncilView, ProgramError> {
    // These floors are the reviewed V3 production and loopback profiles.
    let (review, delay, margin) = if development {
        (750, 150, 150)
    } else {
        (1_512_000, 150, 9_000)
    };
    let bad = || ProgramError::from(OracleError::InvalidGovernanceGateData);
    let (config, bump) =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"council"], controller);
    if owner != controller
        || *key != config
        || data.len() != 384
        || &data[..8] != b"AG3CFG01"
        || data[8] != 1
        || data[9] != bump
        || data[10] != 1
        || data[347..] != [0; 37]
    {
        return Err(bad());
    }
    let pk = |i| Pubkey::new_from_array(data[i..i + 32].try_into().unwrap());
    let n = |i| u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
    let authority =
        Pubkey::find_program_address(&[b"ameba-governance-v3", b"authority"], controller).0;
    let programdata = Pubkey::find_program_address(
        &[controller.as_ref()],
        &solana_sdk_ids::bpf_loader_upgradeable::ID,
    )
    .0;
    if pk(11) != *controller
        || pk(43) != programdata
        || pk(75) != authority
        || pk(107) == Pubkey::default()
        || [*key, programdata, *controller, authority].contains(&pk(107))
        || n(299) == 0
        || n(307) == 0
        || n(339) == 0
        || n(315) < review
        || n(323) < delay
        || n(331) <= n(315).max(n(323)).checked_add(margin).ok_or_else(bad)?
    {
        return Err(bad());
    }
    let seats = core::array::from_fn(|i| pk(139 + 32 * i));
    for (i, seat) in seats.iter().enumerate() {
        if crate::pubkey_is_default(seat) || *seat == authority || seats[..i].contains(seat) {
            return Err(bad());
        }
    }
    let digest = hashv(&[b"amoeba-council-seats-v1", &data[139..299]]).to_bytes();
    Ok(CouncilView {
        epoch: n(299),
        seats,
        digest,
    })
}

#[cfg(not(feature = "localnet-dev-authority"))]
pub(super) fn pinned_controller() -> Result<Pubkey, ProgramError> {
    Ok(crate::governance_gate::PINNED_CONTROLLER_PROGRAM_ID)
}
/// The localnet dev profile reads the same Config layout from this program's own PDA
/// (`[ameba-governance-v3, council]` under the program id), written by tag 243.
#[cfg(feature = "localnet-dev-authority")]
pub(super) fn pinned_controller() -> Result<Pubkey, ProgramError> {
    Ok(crate::id())
}
#[inline(never)]
pub(in crate::processor) fn council(info: &AccountInfo) -> Result<CouncilView, ProgramError> {
    if info.is_signer || info.is_writable || info.executable {
        return Err(OracleError::InvalidAccountList.into());
    }
    decode_council(
        &pinned_controller()?,
        info.key,
        info.owner,
        &info.try_data()?,
    )
}
