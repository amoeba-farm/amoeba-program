//! Exhaustive governance classification for this program's first-byte ABI.
//!
//! The instruction registry remains authoritative: classification calls its actual `from_byte`
//! decoder. Every byte classifies as exactly one of the variants below, and only the two
//! mutating classes ever reach the governance envelope or a handler.

use crate::instruction::OracleInstructionTag;

/// Bytes assigned to oracle instructions that this build does not implement yet. They reject
/// exactly like an unassigned byte, and no other instruction may claim them.
pub const RESERVED_INSTRUCTION_TAGS: [u8; 1] = [205];

/// Oracle v2, heads on pikes (`docs/heads.md`): routed only by builds with the `heads` feature
/// (a default feature, so the production artifact routes it; the dev profiles and the chain enable
/// it too). A `--no-default-features` build classifies it as unassigned, so it rejects before any
/// account is read.
pub const HEADS_INSTRUCTION_TAG: u8 = 210;

/// Bytes of the localnet dev bootstrap (`localnet-dev-authority`, `localnet-governance`) and its
/// month reset (245). Every other build classifies them as unassigned.
pub const DEV_INSTRUCTION_TAGS: [u8; 6] = [240, 241, 242, 243, 244, 245];

/// Bytes routed only by an explicitly selected feature build: the heads family and the dev
/// bootstrap.
pub const FEATURE_GATED_INSTRUCTION_TAGS: [u8; 7] = [
    HEADS_INSTRUCTION_TAG,
    DEV_INSTRUCTION_TAGS[0],
    DEV_INSTRUCTION_TAGS[1],
    DEV_INSTRUCTION_TAGS[2],
    DEV_INSTRUCTION_TAGS[3],
    DEV_INSTRUCTION_TAGS[4],
    DEV_INSTRUCTION_TAGS[5],
];

/// The v1 monthly formation (feature `v1-formation`): instructions that only form a v1 month or
/// serve one mainnet no longer has, routed only by builds that form months (the dev profiles, the
/// test-sbf build, the chain). Mainnet forms no more v1 months (quarters go through heads, 210/28),
/// so the production artifact classifies these bytes as unassigned and refuses them before any
/// account is read. Groups: one-time setup that already ran on mainnet (0, 90, 128, 154); month
/// creation (1, 110, 181, 190); reward budgets (155-158); proposals and coverage (182, 183, 184,
/// 186, 196); source challenges (161, 185, 192); openings and the move to Game (80, 81, 162-165);
/// recipe weights and the recipe index (84, 86, 187, 200); active weights (121, 122, 124); the CFM
/// launch (130); CloseOracleMonth (59; 30/24 closes a month itself). Each runs before its month
/// lists, except 200 (any phase but Closed): mainnet's Live months have complete recipe indexes.
/// Ascending.
pub const V1_FORMATION_INSTRUCTION_TAGS: [u8; 35] = [
    0, 1, 59, 80, 81, 84, 86, 90, 110, 121, 122, 124, 128, 130, 154, 155, 156, 157, 158, 161, 162,
    163, 164, 165, 181, 182, 183, 184, 185, 186, 187, 190, 192, 196, 200,
];

/// Tag 30 selectors of the v1 formation, routed only with `v1-formation`: carry formation (0-7),
/// the median certificates (8-13: 197 reads one only for a source of more than 32 observations or
/// a bucket of more than eight eligible sources; mainnet's Live months have one source per bucket
/// and a few observations each, and its v1 sources take no more) and the evidence backfills (16,
/// 17). Without the feature they reject like an unassigned selector. Selector 22's Activate (an
/// underwritten source's proposal) rejects the same way; its reconcile, bounty and reserve
/// withdrawal stay, as money exits of any agreement that exists. Ascending.
pub const V1_FORMATION_CARRY_SELECTORS: [u8; 16] =
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17];

/// Which feature-gated families a build routes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureGates {
    /// The localnet dev bootstrap (240-245).
    pub dev: bool,
    /// The heads family (210).
    pub heads: bool,
    /// The v1 monthly formation (`V1_FORMATION_INSTRUCTION_TAGS`).
    pub v1_formation: bool,
}

impl FeatureGates {
    pub const NONE: Self = Self {
        dev: false,
        heads: false,
        v1_formation: false,
    };
    pub const ALL: Self = Self {
        dev: true,
        heads: true,
        v1_formation: true,
    };

    /// The families the currently compiled build routes.
    pub const fn active() -> Self {
        Self {
            dev: cfg!(any(
                feature = "localnet-dev-authority",
                feature = "localnet-governance"
            )),
            heads: cfg!(feature = "heads"),
            v1_formation: cfg!(feature = "v1-formation"),
        }
    }

    /// Whether `tag` is a feature-gated byte this build routes. Inlined so a build's constant
    /// gates fold away (a build keeps no trace of the gated bytes it does not route).
    #[inline(always)]
    pub fn routes(self, tag: u8) -> bool {
        (self.heads && tag == HEADS_INSTRUCTION_TAG)
            || (self.dev && DEV_INSTRUCTION_TAGS.contains(&tag))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstructionGovernanceClass {
    Unknown,
    Reserved,
    RecognizedReadOnly,
    RecognizedMutating,
    FeatureGatedMutating,
}

impl InstructionGovernanceClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Reserved => "Reserved",
            Self::RecognizedReadOnly => "RecognizedReadOnly",
            Self::RecognizedMutating => "RecognizedMutating",
            Self::FeatureGatedMutating => "FeatureGatedMutating",
        }
    }
}

/// Classify one byte for an explicitly selected build shape.
///
/// The ordinary `from_byte` decoder runs first because it is the executable registry.
/// Exhaustive tests separately prove that feature and reserved bytes never collide with it.
pub fn classify_instruction_tag_for_build(
    tag: u8,
    gates: FeatureGates,
) -> InstructionGovernanceClass {
    if OracleInstructionTag::from_byte(tag).is_some() {
        if !gates.v1_formation && is_v1_formation_tag(tag) {
            InstructionGovernanceClass::Unknown
        } else {
            InstructionGovernanceClass::RecognizedMutating
        }
    } else if gates.routes(tag) {
        InstructionGovernanceClass::FeatureGatedMutating
    } else if RESERVED_INSTRUCTION_TAGS.contains(&tag) {
        InstructionGovernanceClass::Reserved
    } else {
        InstructionGovernanceClass::Unknown
    }
}

const fn byte_set<const N: usize>(bytes: [u8; N]) -> [u64; 4] {
    let mut set = [0u64; 4];
    let mut i = 0;
    while i < N {
        set[(bytes[i] >> 6) as usize] |= 1 << (bytes[i] & 63);
        i += 1;
    }
    set
}

const V1_FORMATION_TAG_SET: [u64; 4] = byte_set(V1_FORMATION_INSTRUCTION_TAGS);
const V1_FORMATION_CARRY_SELECTOR_SET: [u64; 4] = byte_set(V1_FORMATION_CARRY_SELECTORS);

/// Whether `tag` belongs to the v1 formation (`V1_FORMATION_INSTRUCTION_TAGS`).
#[inline(always)]
pub fn is_v1_formation_tag(tag: u8) -> bool {
    V1_FORMATION_TAG_SET[usize::from(tag >> 6)] & (1 << (tag & 63)) != 0
}

/// Whether `selector` is a tag-30 selector of the v1 formation (`V1_FORMATION_CARRY_SELECTORS`).
#[inline(always)]
pub fn is_v1_formation_carry_selector(selector: u8) -> bool {
    V1_FORMATION_CARRY_SELECTOR_SET[usize::from(selector >> 6)] & (1 << (selector & 63)) != 0
}

/// Classify one byte exactly as the currently compiled feature build routes it.
pub fn classify_active_instruction_tag(tag: u8) -> InstructionGovernanceClass {
    if cfg!(feature = "localnet-governance") && tag == 243 {
        return InstructionGovernanceClass::Unknown;
    }
    classify_instruction_tag_for_build(tag, FeatureGates::active())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_has_exactly_one_class_and_registries_never_collide() {
        let mut counts = [0usize; 5];
        for tag in 0u8..=u8::MAX {
            let recognized = OracleInstructionTag::from_byte(tag).is_some();
            let reserved = RESERVED_INSTRUCTION_TAGS.contains(&tag);
            let gated = FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag);
            assert!(
                usize::from(recognized) + usize::from(reserved) + usize::from(gated) <= 1,
                "byte {tag} is claimed twice"
            );
            let class = classify_active_instruction_tag(tag);
            assert_eq!(
                class,
                if cfg!(feature = "localnet-governance") && tag == 243 {
                    InstructionGovernanceClass::Unknown
                } else {
                    classify_instruction_tag_for_build(tag, FeatureGates::active())
                }
            );
            if !gated && !is_v1_formation_tag(tag) {
                assert_eq!(
                    class,
                    classify_instruction_tag_for_build(tag, FeatureGates::ALL)
                );
            }
            counts[match class {
                InstructionGovernanceClass::Unknown => 0,
                InstructionGovernanceClass::Reserved => 1,
                InstructionGovernanceClass::RecognizedReadOnly => 2,
                InstructionGovernanceClass::RecognizedMutating => 3,
                InstructionGovernanceClass::FeatureGatedMutating => 4,
            }] += 1;
        }
        assert_eq!(counts[1], RESERVED_INSTRUCTION_TAGS.len());
        assert_eq!(counts[2], 0, "no read-only instruction is routed");
        let gates = FeatureGates::active();
        assert_eq!(
            counts[3],
            OracleInstructionTag::ALL.len()
                - if gates.v1_formation {
                    0
                } else {
                    V1_FORMATION_INSTRUCTION_TAGS.len()
                }
        );
        assert_eq!(
            counts[4],
            if gates.dev {
                DEV_INSTRUCTION_TAGS.len() - usize::from(cfg!(feature = "localnet-governance"))
            } else {
                0
            } + usize::from(gates.heads)
        );
        assert_eq!(counts.iter().sum::<usize>(), 256);
        let mut sorted = RESERVED_INSTRUCTION_TAGS;
        sorted.sort_unstable();
        assert_eq!(
            sorted, RESERVED_INSTRUCTION_TAGS,
            "reserved bytes stay sorted"
        );
        assert!(sorted.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_heads_byte_routes_only_where_heads_is_compiled_in() {
        assert!(OracleInstructionTag::from_byte(HEADS_INSTRUCTION_TAG).is_none());
        assert!(!RESERVED_INSTRUCTION_TAGS.contains(&HEADS_INSTRUCTION_TAG));
        assert!(!DEV_INSTRUCTION_TAGS.contains(&HEADS_INSTRUCTION_TAG));
        for (gates, expected) in [
            (FeatureGates::NONE, InstructionGovernanceClass::Unknown),
            (
                FeatureGates {
                    dev: true,
                    heads: false,
                    v1_formation: true,
                },
                InstructionGovernanceClass::Unknown,
            ),
            (
                FeatureGates {
                    dev: false,
                    heads: true,
                    v1_formation: true,
                },
                InstructionGovernanceClass::FeatureGatedMutating,
            ),
            (
                FeatureGates::ALL,
                InstructionGovernanceClass::FeatureGatedMutating,
            ),
        ] {
            assert_eq!(
                classify_instruction_tag_for_build(HEADS_INSTRUCTION_TAG, gates),
                expected,
                "{gates:?}"
            );
            // The heads gate never routes a dev byte, nor the dev gate the heads byte.
            for tag in DEV_INSTRUCTION_TAGS {
                assert_eq!(
                    classify_instruction_tag_for_build(tag, gates)
                        == InstructionGovernanceClass::FeatureGatedMutating,
                    gates.dev,
                    "{gates:?} byte {tag}"
                );
            }
        }
        // The compiled build: one without `heads` (`--no-default-features`) rejects 210 as unknown.
        assert_eq!(
            classify_active_instruction_tag(HEADS_INSTRUCTION_TAG),
            if cfg!(feature = "heads") {
                InstructionGovernanceClass::FeatureGatedMutating
            } else {
                InstructionGovernanceClass::Unknown
            }
        );
        // Every dev profile compiles the heads family in.
        if cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance"
        )) {
            assert!(cfg!(feature = "heads"));
        }
    }

    #[test]
    fn v1_formation_bytes_route_only_where_v1_formation_is_compiled_in() {
        for set in [
            &V1_FORMATION_INSTRUCTION_TAGS[..],
            &V1_FORMATION_CARRY_SELECTORS[..],
        ] {
            assert!(set.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
        }
        for byte in 0u8..=u8::MAX {
            assert_eq!(
                is_v1_formation_tag(byte),
                V1_FORMATION_INSTRUCTION_TAGS.contains(&byte),
                "{byte}"
            );
            assert_eq!(
                is_v1_formation_carry_selector(byte),
                V1_FORMATION_CARRY_SELECTORS.contains(&byte),
                "{byte}"
            );
        }
        for tag in V1_FORMATION_INSTRUCTION_TAGS {
            // An assigned registry byte, never a feature or reserved one.
            assert!(OracleInstructionTag::from_byte(tag).is_some(), "{tag}");
            assert!(!FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag));
            for (gates, expected) in [
                (FeatureGates::NONE, InstructionGovernanceClass::Unknown),
                (
                    FeatureGates {
                        dev: true,
                        heads: true,
                        v1_formation: false,
                    },
                    InstructionGovernanceClass::Unknown,
                ),
                (
                    FeatureGates {
                        dev: false,
                        heads: false,
                        v1_formation: true,
                    },
                    InstructionGovernanceClass::RecognizedMutating,
                ),
                (
                    FeatureGates::ALL,
                    InstructionGovernanceClass::RecognizedMutating,
                ),
            ] {
                assert_eq!(
                    classify_instruction_tag_for_build(tag, gates),
                    expected,
                    "{gates:?} byte {tag}"
                );
            }
            assert_eq!(
                classify_active_instruction_tag(tag),
                if cfg!(feature = "v1-formation") {
                    InstructionGovernanceClass::RecognizedMutating
                } else {
                    InstructionGovernanceClass::Unknown
                }
            );
        }
        // Every other assigned byte routes in every build; the dev profiles and the test-sbf
        // build form months.
        for tag in OracleInstructionTag::ALL {
            if !is_v1_formation_tag(tag as u8) {
                assert_eq!(
                    classify_instruction_tag_for_build(tag as u8, FeatureGates::NONE),
                    InstructionGovernanceClass::RecognizedMutating
                );
            }
        }
        if cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance",
            feature = "test-sbf"
        )) {
            assert!(cfg!(feature = "v1-formation"));
        }
    }

    #[test]
    fn class_names_are_stable() {
        for (class, name) in [
            (InstructionGovernanceClass::Unknown, "Unknown"),
            (InstructionGovernanceClass::Reserved, "Reserved"),
            (
                InstructionGovernanceClass::RecognizedReadOnly,
                "RecognizedReadOnly",
            ),
            (
                InstructionGovernanceClass::RecognizedMutating,
                "RecognizedMutating",
            ),
            (
                InstructionGovernanceClass::FeatureGatedMutating,
                "FeatureGatedMutating",
            ),
        ] {
            assert_eq!(class.as_str(), name);
        }
    }
}
