use solana_system_interface::MAX_PERMITTED_DATA_LENGTH;
use upgrade_controller::{
    artifact_merkle::{
        MAX_ARTIFACT_BYTES_V1, MAX_ARTIFACT_CHUNKS_V1, MAX_ARTIFACT_PROOF_DEPTH_V1,
        MAX_SELECTED_ARTIFACT_CHUNKS_V1, RELEASE1_ARTIFACT_CHUNK_SIZE_V1,
    },
    release1_state::{
        LOADER_V3_PROGRAMDATA_METADATA_LEN_V1, MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1,
    },
};

const PINNED_RUNTIME_MAX_RAW_PROGRAMDATA_BYTES: u64 = MAX_PERMITTED_DATA_LENGTH;
const PINNED_RUNTIME_MAX_PAYLOAD_BYTES: u64 =
    PINNED_RUNTIME_MAX_RAW_PROGRAMDATA_BYTES - LOADER_V3_PROGRAMDATA_METADATA_LEN_V1;

#[test]
fn pinned_runtime_capacity_exceeds_every_published_v1_atomic_ceiling() {
    assert_eq!(PINNED_RUNTIME_MAX_RAW_PROGRAMDATA_BYTES, 10_485_760);
    assert_eq!(PINNED_RUNTIME_MAX_PAYLOAD_BYTES, 10_485_715);
    assert_eq!(MAX_ARTIFACT_BYTES_V1, 1_572_864);
    assert_eq!(MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1, 1_572_909);

    assert!(
        std::hint::black_box(PINNED_RUNTIME_MAX_PAYLOAD_BYTES)
            > std::hint::black_box(MAX_ARTIFACT_BYTES_V1)
    );
    assert!(
        std::hint::black_box(PINNED_RUNTIME_MAX_RAW_PROGRAMDATA_BYTES)
            > std::hint::black_box(MAX_ATOMIC_RAW_PROGRAMDATA_ACCOUNT_BYTES_V1)
    );
}

#[test]
fn legacy_programdata_verification_geometry_cannot_cover_runtime_max() {
    let runtime_chunks =
        PINNED_RUNTIME_MAX_PAYLOAD_BYTES.div_ceil(u64::from(RELEASE1_ARTIFACT_CHUNK_SIZE_V1));
    let padded_chunks = runtime_chunks.next_power_of_two();
    let proof_depth = padded_chunks.trailing_zeros() as usize;

    assert_eq!(runtime_chunks, 640);
    assert_eq!(padded_chunks, 1_024);
    assert_eq!(proof_depth, 10);
    assert_eq!(MAX_SELECTED_ARTIFACT_CHUNKS_V1, 96);
    assert_eq!(MAX_ARTIFACT_CHUNKS_V1, 512);
    assert_eq!(MAX_ARTIFACT_PROOF_DEPTH_V1, 7);

    assert!(runtime_chunks as usize > MAX_SELECTED_ARTIFACT_CHUNKS_V1);
    assert!(runtime_chunks as usize > MAX_ARTIFACT_CHUNKS_V1);
    assert!(proof_depth > MAX_ARTIFACT_PROOF_DEPTH_V1);
}

#[test]
fn an_over_ceiling_monotonic_extension_cannot_reenter_the_legacy_domain() {
    let first_capacity_outside_legacy = MAX_ARTIFACT_BYTES_V1 + 1;
    assert!(first_capacity_outside_legacy <= PINNED_RUNTIME_MAX_PAYLOAD_BYTES);

    // Loader-v3 exposes grow-only extension and Upgrade preserves allocated
    // ProgramData length. Once a valid extension crosses the legacy ceiling,
    // no later nonnegative delta can restore a legacy-admissible capacity.
    for later_delta in [0, 1, 16_384, 1_048_576] {
        let later_capacity = first_capacity_outside_legacy + later_delta;
        assert!(later_capacity > MAX_ARTIFACT_BYTES_V1);
    }
}
