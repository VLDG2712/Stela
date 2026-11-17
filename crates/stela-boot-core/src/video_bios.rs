//! What the boot checks of a video BIOS image it gives a GPU's driver as
//! the function's platform ROM, and where it places an expansion ROM BAR
//! firmware left unassigned.
//!
//! amdgpu takes an image only when it is an ATOM BIOS (`check_atom_bios`);
//! a PCI expansion ROM names the device it is for in its PCI data
//! structure (PCI Firmware 3.0, 5.1.2). The ROM BAR is placed as Linux's
//! PCI core places an unassigned resource: aligned to its size, in the
//! upstream bridge's non-prefetchable window, clear of every BAR and window
//! decoded below that bridge (`pci_assign_resource`, `pbus_size_mem`).
use core::ops::Range;

/// Whether `image` is an ATOM BIOS as `check_atom_bios` judges one of
/// `image.len()` bytes: the ROM signature, a header pointer at 0x48 and
/// "ATOM" (or "MOTA") four bytes after it.
pub fn atom(image: &[u8]) -> bool {
    if image.len() < 0x49 || image[0] != 0x55 || image[1] != 0xaa {
        return false;
    }
    let header = usize::from(u16::from_le_bytes([image[0x48], image[0x49]]));
    if header == 0 {
        return false;
    }
    let at = header + 4;
    image
        .get(at..at + 4)
        .is_some_and(|word| word == b"ATOM" || word == b"MOTA")
}

/// The vendor and device IDs the first image of expansion ROM `image`
/// names in its PCI data structure, if it has one.
pub fn names(image: &[u8]) -> Option<(u16, u16)> {
    if image.get(..2)? != [0x55, 0xaa] {
        return None;
    }
    let at = usize::from(u16::from_le_bytes([*image.get(0x18)?, *image.get(0x19)?]));
    let data = image.get(at..at + 8)?;
    if &data[..4] != b"PCIR" {
        return None;
    }
    Some((
        u16::from_le_bytes([data[4], data[5]]),
        u16::from_le_bytes([data[6], data[7]]),
    ))
}

/// The bytes an expansion ROM BAR decodes, from what it reads back after
/// all ones were written to its address bits (`pci_size`): `None` for a
/// function without one.
pub fn rom_size(read_back: u32) -> Option<u32> {
    let mask = read_back & 0xffff_f800;
    (mask != 0).then(|| (!mask).wrapping_add(1))
}

/// A bridge's non-prefetchable memory window, from its register 0x20
/// (memory base and limit); `None` where it forwards none.
pub fn bridge_window(word: u32) -> Option<Range<u64>> {
    let base = u64::from(word & 0xfff0) << 16;
    let limit = (u64::from(word >> 16 & 0xfff0) << 16) | 0xf_ffff;
    (base <= limit).then(|| base..limit + 1)
}

/// The lowest place for `bytes` (a power of two) aligned to its size in
/// `window` that overlaps none of `used`, as Linux places a resource it
/// assigns in its bridge's window.
pub fn place(window: &Range<u64>, used: &[Range<u64>], bytes: u64) -> Option<u64> {
    if bytes == 0 || !bytes.is_power_of_two() {
        return None;
    }
    let mut start = window.start.next_multiple_of(bytes);
    while start.checked_add(bytes)? <= window.end {
        let end = start + bytes;
        match used
            .iter()
            .filter(|range| range.start < end && start < range.end)
            .map(|range| range.end)
            .max()
        {
            None => return Some(start),
            Some(past) => start = past.next_multiple_of(bytes),
        }
    }
    None
}

#[cfg(test)]
mod tests;
