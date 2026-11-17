//! The ACPI methods of AMD's graphics interface, which amdgpu calls where
//! Linux builds its ACPI support (`amdgpu_bios.c`, `amdgpu_atpx_handler.c`,
//! `amdgpu_acpi.c`). Stela's Linux profile does not build it, so the boot
//! calls the two a hybrid laptop's discrete GPU needs to be usable, as
//! amdgpu calls them, and names the rest once:
//!
//! - `ATRM` hands out the discrete GPU's video BIOS in 4 KiB pieces
//!   (`amdgpu_atrm_get_bios`); [`atrm`] reads it during the PCI census and
//!   the claim gives it to Linux as the function's platform ROM.
//! - `ATPX` function 2 powers the discrete GPU (`amdgpu_atpx_set_discrete_state`);
//!   [`atpx_power_on`] turns it on before the PCI census where it is off and
//!   its port has no power resources of its own.
//! - `ATPX`'s mux switching and runtime power-off, `ATIF` and `ATCS` are not
//!   called: [`METHODS`] says what amdgpu does without them.
use super::native::{Bridge, companion};
use crate::bus::pci::Address;
use crate::platform::acpi::{Argument, Error, Handle, Session, Value};
use alloc::{string::String, vec::Vec};

/// Each method the boot names once where the namespace holds it, and what
/// amdgpu does without it.
pub const METHODS: [(&str, &str); 3] = [
    (
        "ATPX",
        "the boot only powers the discrete GPU on through it when it is off; Linux's runtime \
         power-off and display mux switching through it are not done, so the discrete GPU \
         stays powered and the mux stays as firmware set it",
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

/// `ATPX` functions and the bits of their answers (`amd_acpi.h`).
const VERIFY_INTERFACE: u64 = 0;
const GET_PX_PARAMETERS: u64 = 1;
const POWER_CONTROL: u64 = 2;
const GET_PX_PARAMETERS_SUPPORTED: u32 = 1 << 0;
const POWER_CONTROL_SUPPORTED: u32 = 1 << 1;
const DYNAMIC_PX_SUPPORTED: u32 = 1 << 7;
const DYNAMIC_DGPU_POWER_OFF_SUPPORTED: u32 = 1 << 10;
const MS_HYBRID_GFX_SUPPORTED: u32 = 1 << 14;

/// What powering the discrete GPU on through `ATPX` did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AtpxPower {
    /// The namespace has no `ATPX`.
    Absent,
    /// `ATPX` at this path does not control the discrete GPU's power.
    NoPowerControl(String),
    /// `ATPX` at this path turned the discrete GPU on.
    On(String),
    /// A call failed with this ACPICA status.
    Failed(String, u32),
}

/// Call `ATPX` function `function` with `parameters` (or the fake second
/// integer amdgpu passes without), as `amdgpu_atpx_call` does.
fn atpx_call(
    device: Handle<'_>,
    function: u64,
    parameters: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    let second = match parameters {
        Some(bytes) => Argument::Buffer(bytes),
        None => Argument::Integer(0),
    };
    match device.evaluate(Some("ATPX"), &[Argument::Integer(function), second])? {
        Some(Value::Buffer(answer)) => Ok(answer),
        _ => Ok(Vec::new()),
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    bytes.get(at..at + 4).map_or(0, |word| {
        u32::from_le_bytes([word[0], word[1], word[2], word[3]])
    })
}

/// Whether the `ATPX` at `device` controls the discrete GPU's power, as
/// `amdgpu_atpx_verify_interface`, `amdgpu_atpx_parse_functions` and
/// `amdgpu_atpx_validate` decide it: the interface's power control bit, or
/// a PX parameter that implies it (dynamic PX, dynamic power-off, or
/// hybrid graphics, which amdgpu powers through `ATPX` where the port has
/// no power management of its own, the case this is called for).
pub fn atpx_controls_power(device: Handle<'_>) -> Result<bool, Error> {
    // `struct atpx_verify_interface`: size, version, supported functions.
    let interface = atpx_call(device, VERIFY_INTERFACE, None)?;
    let functions = u32_at(&interface, 4);
    let mut control = functions & POWER_CONTROL_SUPPORTED != 0;
    if functions & GET_PX_PARAMETERS_SUPPORTED != 0 {
        // `struct atpx_px_params`: size, valid flags, flags.
        let parameters = atpx_call(device, GET_PX_PARAMETERS, None)?;
        let size = parameters
            .get(..2)
            .map_or(0, |size| u16::from_le_bytes([size[0], size[1]]));
        if size >= 10 {
            let valid = u32_at(&parameters, 2) & u32_at(&parameters, 6);
            control |= valid
                & (DYNAMIC_PX_SUPPORTED
                    | DYNAMIC_DGPU_POWER_OFF_SUPPORTED
                    | MS_HYBRID_GFX_SUPPORTED)
                != 0;
        }
    }
    Ok(control)
}

/// Turn the discrete GPU on through the first `ATPX` in the namespace that
/// controls its power: `struct atpx_power_control` of 3 bytes with state 1,
/// as `amdgpu_atpx_set_discrete_state(atpx, ATPX_DISCRETE_GPU)`.
pub fn atpx_power_on(session: &Session) -> Result<AtpxPower, Error> {
    let Some(device) = session
        .all_devices()?
        .into_iter()
        .find(|device| device.child("ATPX").is_ok())
    else {
        return Ok(AtpxPower::Absent);
    };
    let path = device.path().unwrap_or_default();
    match atpx_controls_power(device) {
        Ok(true) => {}
        Ok(false) => return Ok(AtpxPower::NoPowerControl(path)),
        Err(error) => return Ok(AtpxPower::Failed(path, error.0)),
    }
    Ok(match atpx_call(device, POWER_CONTROL, Some(&[3, 0, 1])) {
        Ok(_) => AtpxPower::On(path),
        Err(error) => AtpxPower::Failed(path, error.0),
    })
}

#[cfg(test)]
mod tests;
