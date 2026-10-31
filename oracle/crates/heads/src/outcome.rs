//! Who gets what when challenges resolve (spec §2, §4).
//!
//! Any number of challenges may be open against one claim or one source; they resolve
//! together, and the accounting runs exactly once however many won. The only transfers are:
//! the losing claim's bond to the winning challengers; the proposer's slashed delegation to
//! the winners of a source challenge; losing challengers' bonds to the claimant or proposer
//! when the claim or source stands. Every other slash, and every losing challenger's bond when
//! the claim falls anyway, is burned. Nothing slashed ever re-enters a distribution, which is
//! what makes a self-challenge strictly costly.

/// One challenge's bond and whether it won.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub bond: u64,
    pub won: bool,
}

/// Splits `amount` over `weights` pro rata, rounding down; the remainder goes to the last
/// positive weight, so the parts always sum to `amount` (when any weight is positive).
pub fn pro_rata(amount: u64, weights: &[u64]) -> Vec<u64> {
    let total: u128 = weights.iter().map(|&w| u128::from(w)).sum();
    let mut parts = vec![0u64; weights.len()];
    if total == 0 {
        return parts;
    }
    let mut paid = 0u64;
    let mut last = None;
    for (i, &w) in weights.iter().enumerate() {
        if w > 0 {
            parts[i] = (u128::from(amount) * u128::from(w) / total) as u64;
            paid += parts[i];
            last = Some(i);
        }
    }
    if let Some(i) = last {
        parts[i] += amount - paid;
    }
    parts
}

/// The accounting of a resolved claim (spec §2, "Claim challenge").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimResolution {
    /// Whether any challenge won.
    pub falls: bool,
    /// USDC paid to each challenge, in input order: a winner's bond back plus its share of the
    /// claim bond; zero for a loser.
    pub to_challengers: Vec<u64>,
    /// USDC to the claimant: when the claim stands, its own bond back and every challenger's
    /// bond; zero when it falls.
    pub to_claimant: u64,
    /// USDC burned: losing challengers' bonds when the claim falls.
    pub burned: u64,
}

/// Resolves one claim against all of its challenges. A falling claim costs its claimant the bond
/// and nothing else: claim challenges never strike the source and never slash its delegators
/// (the user, 2026-10-29: "just have challenges deal w some of that stuff"; strikes and slashes
/// come only from source challenges, [`resolve_source`]).
pub fn resolve_claim(claim_bond: u64, challenges: &[Challenge]) -> ClaimResolution {
    let falls = challenges.iter().any(|c| c.won);
    if !falls {
        let bonds: u64 = challenges.iter().map(|c| c.bond).sum();
        return ClaimResolution {
            falls,
            to_challengers: vec![0; challenges.len()],
            to_claimant: claim_bond + bonds,
            burned: 0,
        };
    }
    let winning: Vec<u64> = challenges
        .iter()
        .map(|c| if c.won { c.bond } else { 0 })
        .collect();
    let shares = pro_rata(claim_bond, &winning);
    let to_challengers = challenges
        .iter()
        .zip(shares)
        .map(|(c, share)| if c.won { c.bond + share } else { 0 })
        .collect();
    ClaimResolution {
        falls,
        to_challengers,
        to_claimant: 0,
        burned: challenges.iter().filter(|c| !c.won).map(|c| c.bond).sum(),
    }
}

/// The accounting of a resolved source challenge (spec §2, "Source challenge").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceResolution {
    /// Whether any challenge won: the source does not exist, cannot be filled, or publishes
    /// fabricated numbers.
    pub falls: bool,
    /// USDC to each challenge, in input order: a winner's bond back; zero for a loser.
    pub bonds_to_challengers: Vec<u64>,
    /// AMBA to each challenge: the winners' pro rata parts of the proposer's slashed
    /// delegation.
    pub amba_to_challengers: Vec<u64>,
    /// AMBA taken from the proposer's delegation (transferred, not burned).
    pub proposer_slashed: u64,
    /// USDC to the proposer when the source stands: every challenger's bond.
    pub to_proposer: u64,
    /// USDC burned: losing challengers' bonds when the source falls.
    pub burned: u64,
}

/// Resolves a source against all of its source challenges. `proposer_principal` is what the
/// proposer's delegation holds; when the source falls every other delegator is slashed
/// `slash_rate` (burned) and the source takes one strike, whoever fetched.
pub fn resolve_source(
    challenges: &[Challenge],
    proposer_principal: u64,
    proposer_slash_bps: u16,
) -> SourceResolution {
    let falls = challenges.iter().any(|c| c.won);
    if !falls {
        return SourceResolution {
            falls,
            bonds_to_challengers: vec![0; challenges.len()],
            amba_to_challengers: vec![0; challenges.len()],
            proposer_slashed: 0,
            to_proposer: challenges.iter().map(|c| c.bond).sum(),
            burned: 0,
        };
    }
    let proposer_slashed = crate::liability::slash(proposer_principal, proposer_slash_bps);
    let winning: Vec<u64> = challenges
        .iter()
        .map(|c| if c.won { c.bond } else { 0 })
        .collect();
    SourceResolution {
        falls,
        bonds_to_challengers: winning.clone(),
        amba_to_challengers: pro_rata(proposer_slashed, &winning),
        proposer_slashed,
        to_proposer: 0,
        burned: challenges.iter().filter(|c| !c.won).map(|c| c.bond).sum(),
    }
}
