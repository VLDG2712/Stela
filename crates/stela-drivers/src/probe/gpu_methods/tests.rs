//! The real ACPICA over a DSDT with a hybrid laptop's AMD graphics methods.
use super::*;
use crate::platform::acpi::{
    Mode,
    test_support::{TEST_SERIAL, dsdt_fixture},
};
use alloc::{vec, vec::Vec};

/// A package with its one-byte length, which every object below fits.
fn package(op: &[u8], name: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
    let length = u8::try_from(body.len() + 5).unwrap();
    assert!(length < 0x40);
    [op, &[length], name, &body].concat()
}
/// `Method (name, 0) { Return (Zero) }`.
fn method(name: &[u8; 4]) -> Vec<u8> {
    package(&[0x14], name, vec![0, 0xa4, 0])
}
fn device(name: &[u8; 4], body: Vec<Vec<u8>>) -> Vec<u8> {
    package(&[0x5b, 0x82], name, body.concat())
}

#[test]
fn each_method_is_found_once_at_its_first_device() {
    let _serial = TEST_SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // \_SB.GFX0 (the integrated GPU) with ATPX and ATIF, \_SB.DGPU with
    // ATRM and a second ATIF; no ATCS.
    let dsdt = [
        &[0x10][..],
        &[0], // scope length, patched below
        b"\\_SB_",
        &device(b"GFX0", vec![method(b"ATPX"), method(b"ATIF")]),
        &device(b"DGPU", vec![method(b"ATRM"), method(b"ATIF")]),
    ]
    .concat();
    let mut dsdt = dsdt;
    let length = dsdt.len() - 1;
    assert!(length < 0x40);
    dsdt[1] = length as u8;
    let services = dsdt_fixture(&dsdt);
    // SAFETY: the serialized fixture hands its whole provider to the one
    // ACPICA session, which outlives every handle below.
    let session = unsafe { Session::start(services, Mode::InterpreterOnly) }.unwrap();
    let found = find(&session).unwrap();
    let names: Vec<_> = found.iter().map(|(method, _)| *method).collect();
    assert_eq!(names, ["ATPX", "ATRM", "ATIF"]);
    assert!(found[0].1.ends_with("GFX0"), "{found:?}");
    assert!(found[1].1.ends_with("DGPU"), "{found:?}");
    assert!(found[2].1.ends_with("GFX0"), "{found:?}");
}
