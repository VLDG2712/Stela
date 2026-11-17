//! The ACPI methods of AMD's graphics interface, which amdgpu calls where
//! Linux builds its ACPI support (`amdgpu_bios.c`, `amdgpu_atpx_handler.c`,
//! `amdgpu_acpi.c`). Stela's Linux profile does not build it, so the boot
//! calls the one a hybrid laptop's discrete GPU needs for its video BIOS, as
//! amdgpu calls it, and names the rest once:
//!
//! - `ATRM` hands out the discrete GPU's video BIOS in 4 KiB pieces
//!   (`amdgpu_atrm_get_bios`); [`atrm`] reads it during the PCI census and
//!   the claim gives it to Linux as the function's platform ROM.
//! - `ATPX`, `ATIF` and `ATCS` are not called: [`METHODS`] says what amdgpu
//!   does without them.
use super::native::{Bridge, companion};
use crate::bus::pci::Address;
use crate::platform::acpi::{Argument, Error, Handle, Session, Value};
use alloc::{string::String, vec::Vec};

/// Each method the boot names once where the namespace holds it, and what
/// amdgpu does without it.
pub const METHODS: [(&str, &str); 3] = [
    (
        "ATPX",
        "the discrete GPU stays powered and the display mux stays as firmware set it",
    ),
    (
        "ATIF",
        "the platform's display notifications go unheard and amdgpu sets the backlight itself",
    ),
    (
        "ATCS",
        "the PCIe link stays at the speed and width firmware trained",
    ),
];

/// The first device path holding each of [`METHODS`], in that order.
pub fn find(session: &Session) -> Result<Vec<(&'static str, String)>, Error> {
    let devices = session.all_devices()?;
    Ok(METHODS
        .iter()
        .filter_map(|&(method, _)| {
            devices
                .iter()
                .find(|device| device.child(method).is_ok())
                .map(|device| (method, device.path().unwrap_or_default()))
        })
        .collect())
}

/// `ATRM_BIOS_PAGE` and the 256 KiB `amdgpu_atrm_get_bios` reads at most.
const ATRM_PAGE: usize = 4096;
const ATRM_BYTES: usize = 256 * 1024;

/// A video BIOS read through `ATRM`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Atrm {
    /// The display function whose ACPI companion holds the method.
    pub owner: Address,
    /// The method's path.
    pub path: String,
    /// The pieces it returned, in order, up to the first short one.
    pub image: Vec<u8>,
}

/// `ATRM` as `amdgpu_atrm_get_bios` reads it: the first display function
/// of `displays` (class VGA or other display, in census order) whose ACPI
/// companion holds the method, called for each 4 KiB piece until one comes
/// back short or the call fails, at most 256 KiB.
pub(super) fn atrm(
    session: &Session,
    bridges: &[Bridge],
    displays: &[(Address, [u8; 3])],
) -> Option<Atrm> {
    let (owner, method) = displays
        .iter()
        .filter(|(_, class)| class[0] == 3 && matches!(class[1], 0 | 0x80))
        .find_map(|&(address, _)| {
            let device = companion(session, address, bridges)?;
            device.child("ATRM").ok()?;
            Some((address, device))
        })?;
    Some(Atrm {
        owner,
        path: method.path().unwrap_or_default(),
        image: read_atrm(method),
    })
}

/// The pieces `ATRM` under `device` returns, as [`atrm`] reads them.
pub fn read_atrm(device: Handle<'_>) -> Vec<u8> {
    let mut image = Vec::new();
    while image.len() < ATRM_BYTES {
        let offset = image.len() as u64;
        let piece = match device.evaluate(
            Some("ATRM"),
            &[
                Argument::Integer(offset),
                Argument::Integer(ATRM_PAGE as u64),
            ],
        ) {
            Ok(Some(Value::Buffer(piece))) => piece,
            _ => break,
        };
        let length = piece.len().min(ATRM_PAGE);
        image.extend_from_slice(&piece[..length]);
        if length < ATRM_PAGE {
            break;
        }
    }
    image
}

#[cfg(test)]
mod tests;
