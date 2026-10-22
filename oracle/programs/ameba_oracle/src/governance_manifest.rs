//! Exhaustive governance classification for this program's first-byte ABI.
//!
//! The instruction registry remains authoritative: classification calls its actual `from_byte`
//! decoder. Every byte classifies as exactly one of the variants below, and only the two
//! mutating classes ever reach the governance envelope or a handler.

use crate::instruction::OracleInstructionTag;

/// Bytes assigned to oracle instructions that this build does not implement yet. They reject
/// exactly like an unassigned byte, and no other instruction may claim them.
pub const RESERVED_INSTRUCTION_TAGS: [u8; 1] = [205];

/// Bytes routed only by an explicitly selected feature build: the localnet dev bootstrap
/// (`localnet-dev-authority`) and its month reset (245). Every other build classifies them as unassigned.
pub const FEATURE_GATED_INSTRUCTION_TAGS: [u8; 6] = [240, 241, 242, 243, 244, 245];

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
    feature_gated_tags_enabled: bool,
) -> InstructionGovernanceClass {
    if OracleInstructionTag::from_byte(tag).is_some() {
        InstructionGovernanceClass::RecognizedMutating
    } else if feature_gated_tags_enabled && FEATURE_GATED_INSTRUCTION_TAGS.contains(&tag) {
        InstructionGovernanceClass::FeatureGatedMutating
    } else if RESERVED_INSTRUCTION_TAGS.contains(&tag) {
        InstructionGovernanceClass::Reserved
    } else {
        InstructionGovernanceClass::Unknown
    }
}

/// Classify one byte exactly as the currently compiled feature build routes it.
pub fn classify_active_instruction_tag(tag: u8) -> InstructionGovernanceClass {
    if cfg!(feature = "localnet-governance") && tag == 243 {
        return InstructionGovernanceClass::Unknown;
    }
    classify_instruction_tag_for_build(
        tag,
        cfg!(any(
            feature = "localnet-dev-authority",
            feature = "localnet-governance"
        )),
    )
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
                    classify_instruction_tag_for_build(
                        tag,
                        cfg!(any(
                            feature = "localnet-dev-authority",
                            feature = "localnet-governance"
                        )),
                    )
                }
            );
            if !gated {
                assert_eq!(class, classify_instruction_tag_for_build(tag, true));
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
        assert_eq!(counts[3], OracleInstructionTag::ALL.len());
        assert_eq!(
            counts[4],
            if cfg!(any(
                feature = "localnet-dev-authority",
                feature = "localnet-governance"
            )) {
                FEATURE_GATED_INSTRUCTION_TAGS.len()
                    - usize::from(cfg!(feature = "localnet-governance"))
            } else {
                0
            }
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
