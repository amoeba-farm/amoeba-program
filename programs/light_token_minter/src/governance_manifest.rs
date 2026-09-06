//! Exhaustive governance classification for the target program's first-byte ABI.
//!
//! The ordinary registries remain authoritative: classification calls their actual
//! `from_byte` decoders. The Devnet backfill metadata lives here even when that feature is
//! disabled so the checked manifest can prove both supported build shapes from one source.

use crate::{ameba_dlmm_instruction::AmoebaDlmmInstructionTag, instruction::VaultInstructionTag};

pub const PHASE3_TARGET_SOURCE_BASELINE: &str = "1b2230d96e51f6582155d8284900fbfc11ff1f18";
pub const RESERVED_INSTRUCTION_TAGS: [u8; 3] = [249, 250, 251];

pub const DEVNET_SOLO_BACKFILL_INITIALIZE_SIDECAR_TAG: u8 = 29;
pub const DEVNET_SOLO_BACKFILL_INITIALIZE_COHORT_TAG: u8 = 31;
pub const DEVNET_SOLO_BACKFILL_CREATE_SUPPORTED_SOURCE_TAG: u8 = 34;
pub const DEVNET_SOLO_BACKFILL_FINALIZE_COVERAGE_TAG: u8 = 35;
pub const DEVNET_SOLO_BACKFILL_ACCUMULATE_RECIPE_TAG: u8 = 69;
pub const DEVNET_SOLO_BACKFILL_FINALIZE_RECIPE_TAG: u8 = 97;
pub const DEVNET_SOLO_BACKFILL_SUBMIT_OPENING_TAG: u8 = 189;
pub const DEVNET_SOLO_BACKFILL_FINALIZE_OPENING_TAG: u8 = 204;
pub const DEVNET_SOLO_BACKFILL_BEGIN_ACTIVE_WEIGHTS_TAG: u8 = 206;
pub const DEVNET_SOLO_BACKFILL_ACCUMULATE_ACTIVE_WEIGHT_TAG: u8 = 211;
pub const DEVNET_SOLO_BACKFILL_FINALIZE_ACTIVE_WEIGHTS_TAG: u8 = 212;
pub const DEVNET_SOLO_BACKFILL_FINALIZE_GAME_TAG: u8 = 214;

pub const DEVNET_SOLO_BACKFILL_2026_TAG_BYTES: [u8; 12] = [
    DEVNET_SOLO_BACKFILL_INITIALIZE_SIDECAR_TAG,
    DEVNET_SOLO_BACKFILL_INITIALIZE_COHORT_TAG,
    DEVNET_SOLO_BACKFILL_CREATE_SUPPORTED_SOURCE_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_COVERAGE_TAG,
    DEVNET_SOLO_BACKFILL_ACCUMULATE_RECIPE_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_RECIPE_TAG,
    DEVNET_SOLO_BACKFILL_SUBMIT_OPENING_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_OPENING_TAG,
    DEVNET_SOLO_BACKFILL_BEGIN_ACTIVE_WEIGHTS_TAG,
    DEVNET_SOLO_BACKFILL_ACCUMULATE_ACTIVE_WEIGHT_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_ACTIVE_WEIGHTS_TAG,
    DEVNET_SOLO_BACKFILL_FINALIZE_GAME_TAG,
];

#[cfg(not(target_os = "solana"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureInstructionTagMetadata {
    pub name: &'static str,
    pub byte: u8,
}

#[cfg(not(target_os = "solana"))]
pub const DEVNET_SOLO_BACKFILL_2026_TAGS: [FeatureInstructionTagMetadata; 12] = [
    FeatureInstructionTagMetadata {
        name: "InitializeSidecar",
        byte: DEVNET_SOLO_BACKFILL_INITIALIZE_SIDECAR_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "InitializeCohort",
        byte: DEVNET_SOLO_BACKFILL_INITIALIZE_COHORT_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "CreateSupportedSource",
        byte: DEVNET_SOLO_BACKFILL_CREATE_SUPPORTED_SOURCE_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "FinalizeCoverage",
        byte: DEVNET_SOLO_BACKFILL_FINALIZE_COVERAGE_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "AccumulateRecipe",
        byte: DEVNET_SOLO_BACKFILL_ACCUMULATE_RECIPE_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "FinalizeRecipe",
        byte: DEVNET_SOLO_BACKFILL_FINALIZE_RECIPE_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "SubmitOpening",
        byte: DEVNET_SOLO_BACKFILL_SUBMIT_OPENING_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "FinalizeOpening",
        byte: DEVNET_SOLO_BACKFILL_FINALIZE_OPENING_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "BeginActiveWeights",
        byte: DEVNET_SOLO_BACKFILL_BEGIN_ACTIVE_WEIGHTS_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "AccumulateActiveWeight",
        byte: DEVNET_SOLO_BACKFILL_ACCUMULATE_ACTIVE_WEIGHT_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "FinalizeActiveWeights",
        byte: DEVNET_SOLO_BACKFILL_FINALIZE_ACTIVE_WEIGHTS_TAG,
    },
    FeatureInstructionTagMetadata {
        name: "FinalizeGame",
        byte: DEVNET_SOLO_BACKFILL_FINALIZE_GAME_TAG,
    },
];

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

pub fn is_devnet_solo_backfill_2026_instruction_tag(tag: u8) -> bool {
    DEVNET_SOLO_BACKFILL_2026_TAG_BYTES.contains(&tag)
}

/// Classify one byte for an explicitly selected supported build shape.
///
/// The two ordinary `from_byte` decoders run first because they are the executable registries.
/// Exhaustive tests separately prove that feature and reserved bytes never collide with them.
pub fn classify_instruction_tag_for_build(
    tag: u8,
    devnet_solo_backfill_2026: bool,
) -> InstructionGovernanceClass {
    if VaultInstructionTag::from_byte(tag).is_some()
        || AmoebaDlmmInstructionTag::from_byte(tag).is_some()
    {
        InstructionGovernanceClass::RecognizedMutating
    } else if devnet_solo_backfill_2026 && is_devnet_solo_backfill_2026_instruction_tag(tag) {
        InstructionGovernanceClass::FeatureGatedMutating
    } else if RESERVED_INSTRUCTION_TAGS.contains(&tag) {
        InstructionGovernanceClass::Reserved
    } else {
        InstructionGovernanceClass::Unknown
    }
}

/// Classify one byte exactly as the currently compiled feature build routes it.
pub fn classify_active_instruction_tag(tag: u8) -> InstructionGovernanceClass {
    classify_instruction_tag_for_build(tag, cfg!(feature = "devnet-solo-backfill-2026"))
}

#[cfg(not(target_os = "solana"))]
fn tag_identity(tag: u8) -> (String, &'static str, &'static str) {
    if let Some(vault_tag) = VaultInstructionTag::from_byte(tag) {
        return (
            format!("{vault_tag:?}"),
            "VaultInstructionTag::from_byte",
            "always",
        );
    }
    if let Some(dlmm_tag) = AmoebaDlmmInstructionTag::from_byte(tag) {
        return (
            format!("{dlmm_tag:?}"),
            "AmoebaDlmmInstructionTag::from_byte",
            "always",
        );
    }
    if let Some(metadata) = DEVNET_SOLO_BACKFILL_2026_TAGS
        .iter()
        .find(|metadata| metadata.byte == tag)
    {
        return (
            metadata.name.to_owned(),
            "devnet_solo_backfill_2026::canonical_tag_set",
            "devnet-solo-backfill-2026",
        );
    }
    if RESERVED_INSTRUCTION_TAGS.contains(&tag) {
        return (
            format!("Reserved{tag}"),
            "explicit_reserved_bytes",
            "always-reserved",
        );
    }
    ("Unassigned".to_owned(), "unassigned", "none")
}

#[cfg(not(target_os = "solana"))]
fn class_counts(devnet_solo_backfill_2026: bool) -> [usize; 5] {
    let mut counts = [0usize; 5];
    for tag in 0u8..=u8::MAX {
        let index = match classify_instruction_tag_for_build(tag, devnet_solo_backfill_2026) {
            InstructionGovernanceClass::Unknown => 0,
            InstructionGovernanceClass::Reserved => 1,
            InstructionGovernanceClass::RecognizedReadOnly => 2,
            InstructionGovernanceClass::RecognizedMutating => 3,
            InstructionGovernanceClass::FeatureGatedMutating => 4,
        };
        counts[index] += 1;
    }
    counts
}

/// Render the checked Phase 3 manifest without relying on a source parser or handwritten Vault/
/// DLMM name table. This host-only helper is absent from the SBF build.
#[cfg(not(target_os = "solana"))]
pub fn phase3_instruction_manifest_json() -> String {
    use core::fmt::Write as _;

    let default = class_counts(false);
    let feature = class_counts(true);
    let mut output = String::new();
    writeln!(output, "{{").unwrap();
    writeln!(
        output,
        "  \"schema\": \"ameba-spread-phase-3-instruction-manifest-v1\","
    )
    .unwrap();
    writeln!(
        output,
        "  \"target_source_baseline\": \"{PHASE3_TARGET_SOURCE_BASELINE}\","
    )
    .unwrap();
    writeln!(output, "  \"reserved_bytes\": [249, 250, 251],").unwrap();
    writeln!(output, "  \"build_counts\": {{").unwrap();
    writeln!(
        output,
        "    \"default\": {{\"Unknown\": {}, \"Reserved\": {}, \"RecognizedReadOnly\": {}, \"RecognizedMutating\": {}, \"FeatureGatedMutating\": {}}},",
        default[0], default[1], default[2], default[3], default[4]
    )
    .unwrap();
    writeln!(
        output,
        "    \"devnet-solo-backfill-2026\": {{\"Unknown\": {}, \"Reserved\": {}, \"RecognizedReadOnly\": {}, \"RecognizedMutating\": {}, \"FeatureGatedMutating\": {}}}",
        feature[0], feature[1], feature[2], feature[3], feature[4]
    )
    .unwrap();
    writeln!(output, "  }},").unwrap();
    writeln!(output, "  \"tags\": [").unwrap();
    for tag in 0u8..=u8::MAX {
        let (name, registry_source, feature_condition) = tag_identity(tag);
        let trailing = if tag == u8::MAX { "" } else { "," };
        writeln!(
            output,
            "    {{\"byte\": {tag}, \"name\": \"{name}\", \"registry_source\": \"{registry_source}\", \"feature_condition\": \"{feature_condition}\", \"default_class\": \"{}\", \"devnet_backfill_class\": \"{}\"}}{trailing}",
            classify_instruction_tag_for_build(tag, false).as_str(),
            classify_instruction_tag_for_build(tag, true).as_str(),
        )
        .unwrap();
    }
    writeln!(output, "  ]").unwrap();
    writeln!(output, "}}").unwrap();
    output
}
