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
//!
//! An integrated AMD GPU has no option ROM: the host's firmware publishes
//! its video BIOS in the ACPI VFCT table, which Stela reads on bare metal.
//! The run's firmware gets the host's table among its own ACPI tables
//! (`-acpitable`), so the lent GPU finds its video BIOS where it does on the
//! host. Sysfs keeps the table readable by root only: `vfio-gpu.sh` copies
//! it for the run and names the copy in [`HOST_VFCT_VARIABLE`].
use super::Run;
use anyhow::{Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

/// The host's ACPI VFCT table.
const HOST_VFCT: &str = "/sys/firmware/acpi/tables/VFCT";
/// A readable copy of [`HOST_VFCT`], used before it.
const HOST_VFCT_VARIABLE: &str = "STELA_HOST_VFCT";

/// The root port `--vfio-gpu` hangs behind. An explicit slot, past those the
/// other parts number in order, leaves their addresses as they were.
const VFIO_PORT: &str = "pcie-root-port,id=host-gpu-port,chassis=1,slot=1,addr=0x1c";

/// A function of the slot, as sysfs gives it.
#[derive(Clone, Debug)]
pub(super) struct Function {
    pub(super) name: String,
    pub(super) vendor: u16,
    pub(super) device: u16,
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
                device: hex(&directory.join("device"))? as u16,
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

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn half(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Whether the ACPI VFCT `table` holds a video BIOS image for a function
/// with these vendor and device IDs, by the layout amdgpu reads
/// (`UEFI_ACPI_VFCT`, then `VFCT_IMAGE_HEADER`s each followed by its image).
pub(super) fn vfct_holds(table: &[u8], vendor: u16, device: u16) -> bool {
    /// The table's fixed part; the first image's offset is at 0x34.
    const TABLE: usize = 0x4c;
    /// An image's header: its vendor and device IDs at 0xc and 0xe, its
    /// length at 0x18.
    const HEADER: usize = 0x1c;
    if table.len() < TABLE || table[..4] != *b"VFCT" {
        return false;
    }
    let size = (word(table, 4) as usize).min(table.len());
    let mut offset = word(table, 0x34) as usize;
    while offset.checked_add(HEADER).is_some_and(|end| end <= size) {
        let length = word(table, offset + 0x18) as usize;
        // An image that runs past the table ends it, as amdgpu stops there.
        let Some(next) = (offset + HEADER)
            .checked_add(length)
            .filter(|&next| next <= size)
        else {
            return false;
        };
        if length != 0 && half(table, offset + 0xc) == vendor && half(table, offset + 0xe) == device
        {
            return true;
        }
        offset = next;
    }
    false
}

/// QEMU's `-acpitable` value for a whole table in the file at `path`.
pub(super) fn acpi_table_argument(path: &Path) -> String {
    format!("file={}", path.display().to_string().replace(',', ",,"))
}

/// The readable copy of the host's VFCT table, if there is one, and said
/// once when the host has a table this run cannot read.
fn host_vfct() -> Option<(PathBuf, Vec<u8>)> {
    let path = std::env::var_os(HOST_VFCT_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| HOST_VFCT.into());
    match fs::read(&path) {
        Ok(table) => Some((path, table)),
        Err(error) => {
            if path != Path::new(HOST_VFCT) || Path::new(HOST_VFCT).exists() {
                eprintln!(
                    "vfio-gpu: the host's ACPI VFCT table at {} is unreadable ({error}): the lent \
                     GPU gets no video BIOS from it; vfio-gpu.sh names a readable copy in \
                     {HOST_VFCT_VARIABLE}",
                    path.display()
                );
            }
            None
        }
    }
}

/// `--vfio-gpu`'s root port and the card's functions, and the host's VFCT
/// table when it holds the video BIOS of one of them.
pub(super) fn attach(arguments: &Run, command: &mut Process) -> Result<()> {
    if let Some(function) = &arguments.vfio_gpu {
        let devices = Path::new("/sys/bus/pci/devices");
        let functions = vfio_functions(function, devices)?;
        for device in vfio_devices(&functions) {
            command.arg("-device").arg(device);
        }
        if let Some((path, table)) = host_vfct() {
            let slot = slot(function, devices)?;
            if let Some(gpu) = slot.iter().find(|other| {
                functions.contains(&other.name) && vfct_holds(&table, other.vendor, other.device)
            }) {
                eprintln!(
                    "vfio-gpu: the run's ACPI tables carry the host's VFCT ({}), which holds \
                     the video BIOS of {} ({:04x}:{:04x})",
                    path.display(),
                    gpu.name,
                    gpu.vendor,
                    gpu.device
                );
                command.arg("-acpitable").arg(acpi_table_argument(&path));
            }
        }
    }
    Ok(())
}
