//! The real ACPICA over a DSDT with a hybrid laptop's AMD graphics methods.
use super::*;
use crate::platform::acpi::{
    Mode,
    test_support::{TEST_SERIAL, dsdt_fixture},
};
use alloc::{vec, vec::Vec};

/// A package: `op`, its length (which counts itself), then `body`.
fn package(op: &[u8], body: Vec<u8>) -> Vec<u8> {
    let length = if body.len() + 1 < 0x40 {
        vec![(body.len() + 1) as u8]
    } else {
        let total = body.len() + 2;
        assert!(total < 1 << 12);
        vec![0x40 | (total & 0xf) as u8, (total >> 4) as u8]
    };
    [op.to_vec(), length, body].concat()
}
fn device(name: &[u8; 4], body: Vec<Vec<u8>>) -> Vec<u8> {
    package(&[0x5b, 0x82], [name.to_vec(), body.concat()].concat())
}
fn method(name: &[u8; 4], arguments: u8, body: Vec<Vec<u8>>) -> Vec<u8> {
    package(
        &[0x14],
        [name.to_vec(), vec![arguments], body.concat()].concat(),
    )
}
/// `Buffer (size) { bytes }`, the size as a word.
fn buffer(size: u16, bytes: &[u8]) -> Vec<u8> {
    package(
        &[0x11],
        [vec![0x0b], size.to_le_bytes().to_vec(), bytes.to_vec()].concat(),
    )
}
fn ret(value: Vec<u8>) -> Vec<u8> {
    [vec![0xa4], value].concat()
}
/// `If (predicate) { body }`.
fn when(predicate: Vec<u8>, body: Vec<Vec<u8>>) -> Vec<u8> {
    package(&[0xa0], [predicate, body.concat()].concat())
}
/// `LLess (Arg0, value)` and `LEqual (Arg0, value)`, the value as a word.
fn arg0_below(value: u16) -> Vec<u8> {
    [vec![0x95, 0x68, 0x0b], value.to_le_bytes().to_vec()].concat()
}
fn arg0_is(value: u16) -> Vec<u8> {
    [vec![0x93, 0x68, 0x0b], value.to_le_bytes().to_vec()].concat()
}

/// `\_SB.PCI0` (a PCI host bridge) with `GFX0` at 00:02.0, whose `ATRM`
/// returns full pieces below 8 KiB and a short one there; `\_SB.IGPU`'s
/// `ATPX` supports power control and keeps its last power argument in
/// `PWRB`; `\_SB.IGPU` also has `ATIF`.
fn dsdt() -> Vec<u8> {
    let atrm = method(
        b"ATRM",
        2,
        vec![
            when(arg0_below(0x2000), vec![ret(buffer(0x1000, &[0x55, 0xaa]))]),
            ret(buffer(0x10, &[0x01])),
        ],
    );
    let gfx = device(
        b"GFX0",
        vec![
            [
                vec![0x08],
                b"_ADR".to_vec(),
                vec![0x0c],
                0x0002_0000u32.to_le_bytes().to_vec(),
            ]
            .concat(),
            atrm,
        ],
    );
    let host = device(
        b"PCI0",
        vec![
            [
                vec![0x08],
                b"_HID".to_vec(),
                vec![0x0c],
                0x080a_d041u32.to_le_bytes().to_vec(),
            ]
            .concat(),
            gfx,
        ],
    );
    let atpx = method(
        b"ATPX",
        2,
        vec![
            when(arg0_is(0), vec![ret(buffer(8, &[8, 0, 1, 0, 2, 0, 0, 0]))]),
            when(
                arg0_is(2),
                vec![[vec![0x70, 0x69], b"PWRB".to_vec()].concat()],
            ),
            ret(buffer(2, &[0, 0])),
        ],
    );
    let igpu = device(
        b"IGPU",
        vec![
            [vec![0x08], b"PWRB".to_vec(), buffer(3, &[0, 0, 0])].concat(),
            atpx,
            method(b"ATIF", 2, vec![ret(buffer(2, &[0, 0]))]),
        ],
    );
    package(&[0x10], [b"\\_SB_".to_vec(), host, igpu].concat())
}

fn session() -> Session {
    let services = dsdt_fixture(&dsdt());
    // SAFETY: the serialized fixture hands its whole provider to the one
    // ACPICA session, which outlives every handle below.
    unsafe { Session::start(services, Mode::InterpreterOnly) }.unwrap()
}

#[test]
fn each_unserved_method_is_found_once() {
    let _serial = TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = session();
    let found = find(&session).unwrap();
    let names: Vec<_> = found.iter().map(|(method, _)| *method).collect();
    assert_eq!(names, ["ATPX", "ATIF"]);
    assert!(found[0].1.ends_with("IGPU"), "{found:?}");
}

#[test]
fn atrm_is_read_in_pieces_up_to_the_first_short_one() {
    let _serial = TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = session();
    let gfx = Address {
        segment: 0,
        bus: 0,
        device: 2,
        function: 0,
    };
    let audio = Address { function: 1, ..gfx };
    // Only a display function's companion is asked.
    assert_eq!(atrm(&session, &[], &[(audio, [4, 3, 0])]), None);
    let read = atrm(&session, &[], &[(audio, [4, 3, 0]), (gfx, [3, 0, 0])]).unwrap();
    assert_eq!(read.owner, gfx);
    assert!(read.path.ends_with("GFX0"), "{}", read.path);
    assert_eq!(read.image.len(), 2 * ATRM_PAGE + 0x10);
    assert_eq!(read.image[..2], [0x55, 0xaa]);
    assert_eq!(read.image[2 * ATRM_PAGE], 0x01);
}

#[test]
fn atpx_powers_the_discrete_gpu_on_where_it_controls_its_power() {
    let _serial = TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let session = session();
    let powered = atpx_power_on(&session).unwrap();
    let AtpxPower::On(path) = powered else {
        panic!("{powered:?}");
    };
    assert!(path.ends_with("IGPU"), "{path}");
    let kept = session
        .handle("\\_SB.IGPU.PWRB")
        .unwrap()
        .evaluate(None, &[])
        .unwrap();
    assert!(matches!(kept, Some(Value::Buffer(bytes)) if bytes == [3, 0, 1]));
}
