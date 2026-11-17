use super::*;
use alloc::{vec, vec::Vec};

/// A 64 KiB expansion ROM for `vendor:device` with an ATOM header.
pub(crate) fn rom(vendor: u16, device: u16) -> Vec<u8> {
    let mut image = vec![0u8; 0x10000];
    image[0] = 0x55;
    image[1] = 0xaa;
    image[2] = 0x80;
    image[0x18..0x1a].copy_from_slice(&0x200u16.to_le_bytes());
    image[0x200..0x204].copy_from_slice(b"PCIR");
    image[0x204..0x206].copy_from_slice(&vendor.to_le_bytes());
    image[0x206..0x208].copy_from_slice(&device.to_le_bytes());
    image[0x48..0x4a].copy_from_slice(&0x300u16.to_le_bytes());
    image[0x304..0x308].copy_from_slice(b"ATOM");
    image
}

#[test]
fn an_atom_bios_is_known_as_amdgpu_knows_it() {
    let image = rom(0x1002, 0x67df);
    assert!(atom(&image));
    assert_eq!(names(&image), Some((0x1002, 0x67df)));
    let mut legacy = image.clone();
    legacy[0x304..0x308].copy_from_slice(b"MOTA");
    assert!(atom(&legacy));
    let mut other = image.clone();
    other[0x304] = b'X';
    assert!(!atom(&other));
    assert!(!atom(&image[..0x40]));
    let mut unsigned = image;
    unsigned[1] = 0;
    assert!(!atom(&unsigned));
    assert_eq!(names(&unsigned), None);
}

#[test]
fn a_rom_bar_is_sized_and_placed_as_linux_places_it() {
    assert_eq!(rom_size(0xfffe_0000), Some(0x2_0000));
    assert_eq!(rom_size(0xfffe_0001), Some(0x2_0000));
    assert_eq!(rom_size(0), None);
    assert_eq!(bridge_window(0xfcf0_fc00), Some(0xfc00_0000..0xfd00_0000));
    assert_eq!(bridge_window(0x0000_fff0), None);
    let window = 0xfc00_0000..0xfd00_0000;
    // Registers at the bottom, the audio function above them.
    let used = [0xfc00_0000..0xfc08_0000, 0xfc0a_0000..0xfc0a_4000];
    assert_eq!(place(&window, &used, 0x2_0000), Some(0xfc08_0000));
    assert_eq!(place(&window, &used, 0x4_0000), Some(0xfc0c_0000));
    // No room.
    assert_eq!(place(&(0xfc00_0000..0xfc08_0000), &used, 0x2_0000), None);
    assert_eq!(place(&window, &used, 0x3000), None);
}
