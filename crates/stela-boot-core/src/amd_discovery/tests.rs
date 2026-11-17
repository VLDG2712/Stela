use super::*;
use alloc::{vec, vec::Vec};

/// A version 1 binary whose one die lists `ips` (hw_id, major, minor,
/// revision), each with one base address, in an IP discovery table of
/// version `table_version`.
pub(crate) fn binary(ips: &[(u16, u8, u8, u8)], table_version: u16) -> Vec<u8> {
    const TABLE: usize = 64;
    const DIE: usize = TABLE + 80;
    let mut bytes = vec![0u8; DIE + 4 + ips.len() * 12];
    let put16 = |bytes: &mut Vec<u8>, at: usize, value: u16| {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes())
    };
    let put32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
    };
    put32(&mut bytes, 0, BINARY_SIGNATURE);
    put16(&mut bytes, 4, 1);
    let total = bytes.len();
    put16(&mut bytes, 10, total as u16);
    put16(&mut bytes, 12, TABLE as u16);
    put32(&mut bytes, TABLE, DISCOVERY_TABLE_SIGNATURE);
    put16(&mut bytes, TABLE + 4, table_version);
    put16(&mut bytes, TABLE + 6, (total - TABLE) as u16);
    put16(&mut bytes, TABLE + 12, 1);
    put16(&mut bytes, TABLE + 16, DIE as u16);
    put16(&mut bytes, DIE + 2, ips.len() as u16);
    for (index, &(hw_id, major, minor, revision)) in ips.iter().enumerate() {
        let at = DIE + 4 + index * 12;
        put16(&mut bytes, at, hw_id);
        bytes[at + 3] = 1;
        bytes[at + 4] = major;
        bytes[at + 5] = minor;
        bytes[at + 6] = revision;
        put32(&mut bytes, at + 8, 0x1234);
    }
    let table = sum(&bytes[TABLE..]);
    put16(&mut bytes, 14, table);
    let binary = sum(&bytes[10..]);
    put16(&mut bytes, 8, binary);
    bytes
}

/// Raphael's blocks: Ryzen 7000's graphics.
pub(crate) const RAPHAEL: &[(u16, u8, u8, u8)] = &[
    (GC, 10, 3, 6),
    (MP0, 13, 0, 5),
    (MP1, 13, 0, 5),
    (SDMA0, 5, 2, 6),
    (VCN, 3, 1, 2),
    (DMU, 3, 1, 5),
];

#[test]
fn the_table_lists_each_block_with_its_version() {
    let ips = parse(&binary(RAPHAEL, 3)).unwrap();
    assert_eq!(ips.len(), RAPHAEL.len());
    assert_eq!(version(&ips, GC), Some((10, 3, 6)));
    assert_eq!(version(&ips, DMU), Some((3, 1, 5)));
    assert_eq!(version(&ips, VPE), None);
    // VCN's capability bits are not its revision.
    let ips = parse(&binary(&[(VCN, 4, 0, 0x85)], 3)).unwrap();
    assert_eq!(version(&ips, VCN), Some((4, 0, 5)));
}

#[test]
fn a_damaged_table_is_refused() {
    let good = binary(RAPHAEL, 3);
    let mut bad = good.clone();
    bad[0] ^= 1;
    assert_eq!(parse(&bad), Err(Error::Signature));
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(parse(&bad), Err(Error::Checksum));
    assert_eq!(parse(&good[..40]), Err(Error::Truncated));
}

#[test]
fn the_table_is_found_where_amdgpu_looks() {
    let registers = |memsize: u32, scratch: [u32; 3], ready: bool| {
        move |offset: usize| match offset {
            MP0_SMN_C2PMSG_33 => {
                if ready {
                    FIRMWARE_READY
                } else {
                    0
                }
            }
            RCC_CONFIG_MEMSIZE => memsize,
            DRIVER_SCRATCH_0 => scratch[0],
            DRIVER_SCRATCH_1 => scratch[1],
            DRIVER_SCRATCH_2 => scratch[2],
            _ => 0,
        }
    };
    // 8 GiB of VRAM with the table at its usual place.
    let top = (8192u64 << 20) - BELOW_TOP;
    assert_eq!(
        place(registers(8192, [0; 3], true), |at| if at == top {
            BINARY_SIGNATURE
        } else {
            0
        }),
        Ok(Place {
            offset: top,
            bytes: TABLE_BYTES
        })
    );
    // Elsewhere, where the scratch registers say.
    assert_eq!(
        place(registers(8192, [0x1000, 1, 0x3000], true), |_| 0),
        Ok(Place {
            offset: 0x1_0000_1000,
            bytes: 0x3000
        })
    );
    assert_eq!(
        place(registers(8192, [0; 3], false), |_| 0),
        Err(Error::NotReady)
    );
    assert_eq!(
        place(registers(0, [0; 3], true), |_| 0),
        Err(Error::InSystemMemory)
    );
    assert_eq!(
        place(registers(u32::MAX, [0; 3], true), |_| 0),
        Err(Error::NoAnswer)
    );
}
