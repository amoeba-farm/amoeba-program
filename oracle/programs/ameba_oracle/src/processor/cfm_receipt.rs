//! The CFM launch receipt (tag 130, `cfm_launch`) as the council reads it. The launch itself
//! forms a month (`v1-formation`); a launched month's council fast path (operation 2) stays in
//! every build, so it checks the finished receipt here.

use super::*;
use crate::cfm::{self, CfmLaunchReceipt, CfmProduct};

pub(super) fn bad_receipt<T>() -> Result<T, ProgramError> {
    Err(OracleError::InvalidCfmLaunchReceipt.into())
}

pub(super) fn load_receipt(
    program_id: &Pubkey,
    info: &AccountInfo,
    product: &CfmProduct,
) -> Result<CfmLaunchReceipt, ProgramError> {
    let receipt: CfmLaunchReceipt = load_exact_zero_padded_state(
        info,
        program_id,
        CfmLaunchReceipt::LEN,
        OracleError::InvalidCfmLaunchReceipt,
    )?;
    let (key, bump) = cfm::find_receipt_address(program_id, &product.product_id);
    if info.executable
        || *info.key != key
        || receipt.discriminator != cfm::RECEIPT_DISCRIMINATOR
        || receipt.version != cfm::RECEIPT_VERSION
        || receipt.bump != bump
        || receipt.product != product.code
        || receipt.row_count != product.parent_count
        || receipt.cursor > receipt.row_count
        || receipt.finished != (receipt.finished_at != 0)
        || (receipt.finished && receipt.cursor != receipt.row_count)
        || receipt.approving_seats.count_ones() != 3
        || receipt.approving_seats & !0b1_1111 != 0
        || receipt.council_epoch == 0
        || receipt.begun_at == 0
        || crate::bytes32_is_zero(&receipt.plan_hash)
    {
        return bad_receipt();
    }
    Ok(receipt)
}

/// The council's bucket settlement of a launched month may apply early only with this: the
/// finished receipt of the period's product, naming this period and month.
pub(super) fn validate_finished_launch(
    program_id: &Pubkey,
    period_info: &AccountInfo,
    month_key: &Pubkey,
    receipt_info: &AccountInfo,
) -> ProgramResult {
    if receipt_info.is_signer || receipt_info.is_writable {
        return Err(OracleError::InvalidAccountList.into());
    }
    let anchor = load_valid_period_anchor(program_id, period_info)?;
    let product = if anchor.underlying_id == cfm::RAMX.product_id {
        cfm::RAMX
    } else if anchor.underlying_id == cfm::NANDX.product_id {
        cfm::NANDX
    } else {
        return bad_receipt();
    };
    let receipt = load_receipt(program_id, receipt_info, &product)?;
    if !receipt.finished || receipt.period != *period_info.key || receipt.month != *month_key {
        return bad_receipt();
    }
    Ok(())
}
