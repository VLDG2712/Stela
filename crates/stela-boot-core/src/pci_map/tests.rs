use super::*;
use alloc::vec;

/// A root port at 00:01.1 forwarding bus 3 and the window 0xfc00_0000 to
/// 0xfcff_ffff; a GPU at 03:00.0 with its registers at the window's
/// bottom and its option ROM copied at 0x1000_0000; its audio function.
fn map() -> Vec<Function> {
    vec![
        Function {
            device: 1,
            function: 1,
            bridge: Some((3, 3, 0xfcf0_fc00)),
            ..Function::default()
        },
        Function {
            bus: 3,
            spans: vec![0xe000_0000..0xf000_0000, 0xfc00_0000..0xfc04_0000],
            rom: Some(0x1000_0000..0x1001_0000),
            ..Function::default()
        },
        Function {
            bus: 3,
            function: 1,
            spans: core::iter::once(0xfc06_0000..0xfc06_4000).collect(),
            ..Function::default()
        },
    ]
}

#[test]
fn the_map_reads_back_as_written() {
    let map = map();
    assert_eq!(decode(&encode(&map)).as_ref(), Some(&map));
    assert_eq!(decode(&encode(&map)[..10]), None);
    assert_eq!(decode(&[]), None);
}

#[test]
fn an_unassigned_rom_bar_goes_in_its_bridges_window_clear_of_every_bar() {
    let map = map();
    // Between the registers and the audio function, else above both.
    assert_eq!(rom_room(&map, 0, 3, 0x2_0000), Some(0xfc04_0000));
    assert_eq!(rom_room(&map, 0, 3, 0x4_0000), Some(0xfc08_0000));
    // No bridge above bus 0, and none on another segment.
    assert_eq!(rom_room(&map, 0, 0, 0x2_0000), None);
    assert_eq!(rom_room(&map, 1, 3, 0x2_0000), None);
    // A window too small.
    assert_eq!(rom_room(&map, 0, 3, 0x200_0000), None);
}
