//! An AMD GPU's IP discovery table, which names the version of each of its
//! IP blocks, as amdgpu reads it at its probe (`amdgpu_discovery.c`). The
//! boot reads it before the GPU's reset so the firmware it gives amdgpu is
//! the files of those versions only.
//!
//! The table lies in the GPU's memory, 64 KiB below the top of its VRAM as
//! `RCC_CONFIG_MEMSIZE` gives it, or where the GPU's security processor
//! says in the driver scratch registers (`amdgpu_discovery_get_tmr_info`).
//! amdgpu reads it through the `MM_INDEX`/`MM_DATA` window, which needs no
//! mapping of the GPU's memory (`amdgpu_device_mm_access`). The checks here
//! are `amdgpu_discovery_init`'s; the walk is
//! `amdgpu_discovery_reg_base_init`'s.
use alloc::vec::Vec;

/// Registers, as byte offsets in the register BAR (`amdgpu_discovery.c`:
/// "consistent across all the SOCs").
pub const MM_INDEX: usize = 0;
pub const MM_DATA: usize = 4;
pub const MM_INDEX_HI: usize = 0x6 * 4;
pub const DRIVER_SCRATCH_0: usize = 0x94 * 4;
pub const DRIVER_SCRATCH_1: usize = 0x95 * 4;
pub const DRIVER_SCRATCH_2: usize = 0x96 * 4;
pub const RCC_CONFIG_MEMSIZE: usize = 0xde3 * 4;
pub const MP0_SMN_C2PMSG_33: usize = 0x16061 * 4;
/// The register window the read needs.
pub const REGISTER_BYTES: usize = MP0_SMN_C2PMSG_33 + 4;
/// `C2PMSG_33` bit 31: the GPU's firmware finished its start and placed the
/// table.
pub const FIRMWARE_READY: u32 = 1 << 31;

/// `DISCOVERY_TMR_SIZE` and `DISCOVERY_TMR_OFFSET`.
pub const TABLE_BYTES: usize = 10 << 10;
pub const BELOW_TOP: u64 = 64 << 10;
/// The most a scratch-register size may ask to read.
const MOST_BYTES: usize = 64 << 10;

pub const BINARY_SIGNATURE: u32 = 0x2821_1407;
const DISCOVERY_TABLE_SIGNATURE: u32 = 0x5344_4950;

/// The hardware IDs of the blocks whose firmware amdgpu names after their
/// versions (`soc15_hw_ip.h`).
pub const MP1: u16 = 1;
pub const GC: u16 = 11;
pub const VCN: u16 = 12;
pub const VPE: u16 = 21;
pub const SDMA0: u16 = 42;
pub const DMU: u16 = 271;
pub const MP0: u16 = 255;
/// `HW_ID_MAX` and `HWIP_MAX_INSTANCE`: entries past these are skipped.
const HW_ID_MAX: u16 = 300;
const MAX_INSTANCE: u8 = 44;

/// One IP block as the table lists it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ip {
    pub hw_id: u16,
    pub instance: u8,
    pub major: u8,
    pub minor: u8,
    pub revision: u8,
}

/// Why the table was not read or not taken.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// `RCC_CONFIG_MEMSIZE` reads all ones: the GPU does not answer.
    NoAnswer,
    /// The GPU's firmware has not placed the table yet (`C2PMSG_33`).
    NotReady,
    /// The table is in system memory (no VRAM size and no scratch place),
    /// where amdgpu finds it through ACPI.
    InSystemMemory,
    Signature,
    Checksum,
    Version(u16),
    TableSignature,
    TableChecksum,
    Truncated,
    DieId,
}

/// Where to read the table: the VRAM offset and length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Place {
    pub offset: u64,
    pub bytes: usize,
}

/// Where the table lies, as `amdgpu_discovery_get_tmr_info` finds it on a
/// GPU that is no virtual function: `read` reads a register of the
/// register BAR, `signature_at` the dword at a VRAM offset.
pub fn place(
    mut read: impl FnMut(usize) -> u32,
    mut signature_at: impl FnMut(u64) -> u32,
) -> Result<Place, Error> {
    if read(MP0_SMN_C2PMSG_33) & FIRMWARE_READY == 0 {
        return Err(Error::NotReady);
    }
    let vram_mib = read(RCC_CONFIG_MEMSIZE);
    if vram_mib == u32::MAX {
        return Err(Error::NoAnswer);
    }
    if vram_mib != 0 {
        let offset = (u64::from(vram_mib) << 20).saturating_sub(BELOW_TOP);
        if signature_at(offset) == BINARY_SIGNATURE {
            return Ok(Place {
                offset,
                bytes: TABLE_BYTES,
            });
        }
    }
    let size = read(DRIVER_SCRATCH_2);
    if size != 0 {
        return Ok(Place {
            offset: u64::from(read(DRIVER_SCRATCH_1)) << 32 | u64::from(read(DRIVER_SCRATCH_0)),
            bytes: (size as usize).min(MOST_BYTES),
        });
    }
    if vram_mib == 0 {
        return Err(Error::InSystemMemory);
    }
    Ok(Place {
        offset: (u64::from(vram_mib) << 20).saturating_sub(BELOW_TOP),
        bytes: TABLE_BYTES,
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, Error> {
    bytes
        .get(at..at + 2)
        .map(|word| u16::from_le_bytes([word[0], word[1]]))
        .ok_or(Error::Truncated)
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, Error> {
    bytes
        .get(at..at + 4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .ok_or(Error::Truncated)
}

fn sum(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)))
}

/// The IP blocks the table `binary` lists, with its binary and data table
/// checked as amdgpu checks them. VCN's revision loses its two capability
/// bits, as amdgpu masks them.
pub fn parse(binary: &[u8]) -> Result<Vec<Ip>, Error> {
    if u32_at(binary, 0)? != BINARY_SIGNATURE {
        return Err(Error::Signature);
    }
    // The byte sum of the binary after its checksum field, to its size.
    let checksum = u16_at(binary, 8)?;
    let size = usize::from(u16_at(binary, 10)?);
    let body = binary.get(10..size.max(10)).ok_or(Error::Truncated)?;
    if sum(body) != checksum {
        return Err(Error::Checksum);
    }
    // The IP discovery table's entry: version 0 and 1 list tables from 12,
    // version 2 after a count and padding.
    let entry = match u16_at(binary, 4)? {
        0 | 1 => 12,
        2 => 16,
        version => return Err(Error::Version(version)),
    };
    let offset = usize::from(u16_at(binary, entry)?);
    let table_checksum = u16_at(binary, entry + 2)?;
    if u32_at(binary, offset)? != DISCOVERY_TABLE_SIGNATURE {
        return Err(Error::TableSignature);
    }
    let table_bytes = usize::from(u16_at(binary, offset + 6)?);
    let table = binary
        .get(offset..offset + table_bytes)
        .ok_or(Error::Truncated)?;
    if sum(table) != table_checksum {
        return Err(Error::TableChecksum);
    }
    let version = u16_at(binary, offset + 4)?;
    let dies = usize::from(u16_at(binary, offset + 12)?).min(16);
    // After the 16 die entries, version 4's flag byte: 64-bit base
    // addresses.
    let wide = version == 4 && binary.get(offset + 14 + 16 * 4).ok_or(Error::Truncated)? & 1 != 0;
    let mut ips = Vec::new();
    for die in 0..dies {
        let die_offset = usize::from(u16_at(binary, offset + 14 + die * 4 + 2)?);
        if usize::from(u16_at(binary, die_offset)?) != die {
            return Err(Error::DieId);
        }
        let count = u16_at(binary, die_offset + 2)?;
        let mut at = die_offset + 4;
        for _ in 0..count {
            let entry = binary.get(at..at + 8).ok_or(Error::Truncated)?;
            let hw_id = u16::from_le_bytes([entry[0], entry[1]]);
            let instance = entry[2];
            let bases = usize::from(entry[3]);
            let mut revision = entry[6];
            if hw_id == VCN {
                revision &= !0xc0;
            }
            if hw_id < HW_ID_MAX && instance < MAX_INSTANCE {
                ips.push(Ip {
                    hw_id,
                    instance,
                    major: entry[4],
                    minor: entry[5],
                    revision,
                });
            }
            at += 8 + bases * if wide { 8 } else { 4 };
        }
    }
    Ok(ips)
}

/// The version of the first instance of `hw_id`, if the table lists it.
pub fn version(ips: &[Ip], hw_id: u16) -> Option<(u8, u8, u8)> {
    ips.iter()
        .filter(|ip| ip.hw_id == hw_id)
        .min_by_key(|ip| ip.instance)
        .map(|ip| (ip.major, ip.minor, ip.revision))
}

#[cfg(test)]
mod tests;
