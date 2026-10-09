//! Private projector output: callers cannot upload or select these mutations.
use crate::ProgramError;
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub(crate) struct Run {
    pub offset: u32,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub(crate) struct Delta {
    pub role: u8,
    pub data_len: u32,
    pub runs: Vec<Run>,
    pub slots: Vec<u32>,
}
fn invalid() -> ProgramError {
    ProgramError::InvalidAccountData
}
pub(crate) fn diff_allocated(
    role: usize,
    before: &[u8],
    after: &[u8],
    allocation: usize,
    slots: Vec<u32>,
) -> Result<Delta, ProgramError> {
    if before.len() != after.len() {
        return Err(invalid());
    }
    let mut runs: Vec<Run> = vec![];
    let mut n = 0;
    let mut checked_end = 0;
    while n < before.len() {
        // Compare each block once. Retrying an overlapping syscall at every
        // equal byte inside a changed block is much more costly than scanning it.
        if n >= checked_end {
            checked_end = (n + 64).min(before.len());
            if solana_program::program_memory::sol_memcmp(
                &before[n..checked_end],
                &after[n..checked_end],
                checked_end - n,
            ) == 0
            {
                n = checked_end;
                continue;
            }
        }
        if before.len() - n >= 8 {
            // A changed block can still contain long unchanged stretches. Both
            // slices have the same checked length, so these unaligned words
            // are entirely inside initialized bytes at every slice alignment.
            let unchanged = unsafe {
                core::ptr::read_unaligned(before.as_ptr().add(n).cast::<u64>())
                    == core::ptr::read_unaligned(after.as_ptr().add(n).cast::<u64>())
            };
            if unchanged {
                n += 8;
                continue;
            }
        }
        if before[n] == after[n] {
            n += 1;
            continue;
        }
        let start = n;
        n += 1;
        while n < before.len() && before[n] != after[n] {
            n += 1;
        }
        // A separate run costs eight Borsh metadata bytes. Carrying at most
        // eight unchanged bytes is no larger and avoids another allocation and
        // serializer call. The sealed source transcript authenticates them.
        if let Some(last) = runs.last_mut() {
            let end = last.offset as usize + last.bytes.len();
            if start - end <= 8 {
                last.bytes.extend_from_slice(&after[end..n]);
                continue;
            }
        }
        runs.push(Run {
            offset: u32::try_from(start).map_err(|_| invalid())?,
            bytes: after[start..n].to_vec(),
        });
    }
    if before.len() > allocation {
        return Err(invalid());
    }
    let delta = Delta {
        role: u8::try_from(role).map_err(|_| invalid())?,
        data_len: u32::try_from(allocation).map_err(|_| invalid())?,
        runs,
        slots,
    };
    validate(&delta, allocation)?;
    Ok(delta)
}
pub(crate) fn validate(delta: &Delta, len: usize) -> Result<(), ProgramError> {
    if delta.data_len as usize != len {
        return Err(invalid());
    }
    let mut end = 0usize;
    for run in &delta.runs {
        let start = run.offset as usize;
        let next = start.checked_add(run.bytes.len()).ok_or_else(invalid)?;
        if run.bytes.is_empty() || start < end || next > len {
            return Err(invalid());
        }
        end = next;
    }
    end = 0;
    for slot in &delta.slots {
        let start = *slot as usize;
        let next = start.checked_add(8).ok_or_else(invalid)?;
        if start < end || next > len {
            return Err(invalid());
        }
        end = next;
    }
    Ok(())
}
pub(crate) fn apply(delta: &Delta, data: &mut [u8], slot: u64) -> Result<(), ProgramError> {
    validate(delta, data.len())?;
    for run in &delta.runs {
        let start = run.offset as usize;
        data[start..start + run.bytes.len()].copy_from_slice(&run.bytes);
    }
    for offset in &delta.slots {
        let start = *offset as usize;
        data[start..start + 8].copy_from_slice(&slot.to_le_bytes());
    }
    Ok(())
}
