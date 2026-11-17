//! The ACPI methods of AMD's graphics interface that amdgpu calls where
//! Linux builds its ACPI support (`amdgpu_acpi.c`, `amdgpu_atpx_handler.c`),
//! which Stela's Linux profile does not: `ATPX` switches and powers a hybrid
//! laptop's discrete GPU, `ATRM` hands out its video BIOS, `ATIF` carries
//! the platform's display notifications and backlight, `ATCS` the PCIe link
//! requests. Each that the namespace holds is named once, with the
//! response amdgpu then gives without it.
use crate::platform::acpi::{Error, Session};
use alloc::{string::String, vec::Vec};

/// Each method, and what amdgpu does where it is not called.
pub const METHODS: [(&str, &str); 4] = [
    (
        "ATPX",
        "the discrete GPU stays powered and the display mux stays as firmware set it",
    ),
    (
        "ATRM",
        "amdgpu reads the discrete GPU's video BIOS from the VFCT table, the GPU's ROM or its \
         registers instead",
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

#[cfg(test)]
mod tests;
