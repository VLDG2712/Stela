//! `--vfio-gpu`: a host GPU lent to the run through VFIO, behind a root port
//! of its own.
//!
//! VFIO's unit is the IOMMU group: every device of a lent function's group
//! must be held by vfio-pci, and a function may be lent alone when its group
//! holds only it. libvirt likewise assigns the functions its domain names,
//! not their whole slot. A discrete card's slot is the card: its display,
//! HD audio and, on cards with a USB-C port, its USB and UCSI controllers,
//! which share its reset. An integrated GPU's slot also holds functions of
//! the platform the host needs, such as its USB controllers and security
//! processor, each in a group of its own, and a reset of that bus is refused
//! while the host holds them.
use super::Run;
use anyhow::{Result, bail, ensure};
use std::{fs, path::Path, process::Command as Process};

/// The root port `--vfio-gpu` hangs behind. An explicit slot, past those the
/// other parts number in order, leaves their addresses as they were.
const VFIO_PORT: &str = "pcie-root-port,id=host-gpu-port,chassis=1,slot=1,addr=0x1c";

/// A function of the slot, as sysfs gives it.
#[derive(Clone, Debug)]
pub(super) struct Function {
    pub(super) name: String,
    pub(super) vendor: u16,
    /// Base class, subclass and programming interface.
    pub(super) class: u32,
}

impl Function {
    fn display(&self) -> bool {
        self.class >> 16 == 0x03
    }

    fn audio(&self) -> bool {
        self.class >> 8 == 0x0403
    }

    /// A function a GPU card carries: display, HD audio, or a USB-C port's
    /// USB (0x0c03) and UCSI (0x0c80) controllers.
    fn of_a_card(&self) -> bool {
        self.display() || self.audio() || matches!(self.class >> 8, 0x0c03 | 0x0c80)
    }
}

/// The functions of `asked`'s slot that belong to its GPU, the asked one
/// first. The whole slot when every function in it has the display
/// function's vendor and is one a card carries (a discrete card); otherwise
/// only that vendor's display and HD audio functions (an integrated GPU,
/// whose slot also holds the platform's own functions).
pub(super) fn gpu_functions(slot: &[Function], asked: &str) -> Result<Vec<String>> {
    let Some(asked_function) = slot.iter().find(|function| function.name == asked) else {
        bail!("the host has no PCI function {asked}");
    };
    let display = if asked_function.display() {
        asked_function
    } else {
        match slot.iter().find(|function| function.display()) {
            Some(display) => display,
            None => bail!("the slot of {asked} holds no display function"),
        }
    };
    let card = slot
        .iter()
        .all(|function| function.vendor == display.vendor && function.of_a_card());
    let mut functions: Vec<String> = slot
        .iter()
        .filter(|function| {
            card || (function.vendor == display.vendor && (function.display() || function.audio()))
        })
        .map(|function| function.name.clone())
        .collect();
    ensure!(
        functions.iter().any(|name| name == asked),
        "{asked} is not one of the GPU's functions in its slot"
    );
    functions.sort_by_key(|name| (name != asked, name.clone()));
    Ok(functions)
}

fn hex(path: &Path) -> Result<u32> {
    let text = fs::read_to_string(path)?;
    let digits = text.trim().trim_start_matches("0x");
    u32::from_str_radix(digits, 16).map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))
}

/// The functions of `function`'s slot in `devices` (sysfs's PCI devices).
pub(super) fn slot(function: &str, devices: &Path) -> Result<Vec<Function>> {
    let (slot, _) = function.rsplit_once('.').expect("parsed as a PCI function");
    let mut functions = Vec::new();
    for entry in fs::read_dir(devices)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name
            .rsplit_once('.')
            .is_some_and(|(other, _)| other == slot)
        {
            let directory = devices.join(&name);
            functions.push(Function {
                vendor: hex(&directory.join("vendor"))? as u16,
                class: hex(&directory.join("class"))?,
                name,
            });
        }
    }
    Ok(functions)
}

/// `--vfio-gpu`'s functions ([`gpu_functions`]), each of which must be
/// vfio-pci's.
pub(super) fn vfio_functions(function: &str, devices: &Path) -> Result<Vec<String>> {
    let functions = gpu_functions(&slot(function, devices)?, function)?;
    for name in &functions {
        let driver = fs::read_link(devices.join(name).join("driver")).ok();
        ensure!(
            driver.as_deref().and_then(Path::file_name) == Some("vfio-pci".as_ref()),
            "{name} is not bound to vfio-pci: run through xtask/scripts/vfio-gpu.sh"
        );
    }
    Ok(functions)
}

/// The QEMU devices of `functions`, in one multifunction slot of the port.
///
/// No option ROM: Stela drives the card with no firmware help, and a card nothing
/// has posted since it lost power hangs the firmware in the ROM's GOP driver.
pub(super) fn vfio_devices(functions: &[String]) -> Vec<String> {
    let mut devices = vec![VFIO_PORT.to_owned()];
    devices.extend(functions.iter().enumerate().map(|(index, host)| {
        let multifunction = if index == 0 { ",multifunction=on" } else { "" };
        format!("vfio-pci,host={host},bus=host-gpu-port,addr=0.{index},rombar=0{multifunction}")
    }));
    devices
}

/// `--vfio-gpu`'s root port and the card's functions.
pub(super) fn attach(arguments: &Run, command: &mut Process) -> Result<()> {
    if let Some(function) = &arguments.vfio_gpu {
        let functions = vfio_functions(function, Path::new("/sys/bus/pci/devices"))?;
        for device in vfio_devices(&functions) {
            command.arg("-device").arg(device);
        }
    }
    Ok(())
}
