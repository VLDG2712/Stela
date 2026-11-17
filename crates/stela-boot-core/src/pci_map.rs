//! What the firmware's PCI bus driver knew of each function before it
//! exited, as Linux's EFI stub hands some of it over: the memory each BAR
//! decodes, each bridge's buses and non-prefetchable window, and a copy of
//! a display function's option ROM, which the firmware read from the ROM
//! BAR and keeps (`EFI_PCI_IO_PROTOCOL.RomImage`; Linux's stub copies it
//! into `setup_data` as `SETUP_PCI`, and `pci_dev::rom` holds it).
//!
//! The boot reads it once before the firmware's exit and the engine places
//! an expansion ROM BAR the firmware left unassigned with it
//! ([`rom_room`]): the firmware sized every BAR, which the engine cannot
//! do again on functions other drivers own.
use crate::video_bios::{bridge_window, place};
use alloc::vec::Vec;
use core::ops::Range;

/// One function as the firmware left it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Function {
    pub segment: u16,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    /// A bridge's secondary and subordinate buses and register 0x20.
    pub bridge: Option<(u8, u8, u32)>,
    /// The memory each BAR decodes.
    pub spans: Vec<Range<u64>>,
    /// Where the copy of its option ROM lies in memory the boot keeps.
    pub rom: Option<Range<u64>>,
}

/// The most functions the map holds.
pub const MOST_FUNCTIONS: usize = 1024;
const MOST_SPANS: usize = 16;

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes);
}

/// The map as bytes, which [`decode`] reads back.
pub fn encode(functions: &[Function]) -> Vec<u8> {
    let mut out = Vec::new();
    put(
        &mut out,
        &(functions.len().min(MOST_FUNCTIONS) as u32).to_le_bytes(),
    );
    for function in functions.iter().take(MOST_FUNCTIONS) {
        put(&mut out, &function.segment.to_le_bytes());
        put(
            &mut out,
            &[function.bus, function.device, function.function],
        );
        let (secondary, subordinate, window) = function.bridge.unwrap_or_default();
        put(
            &mut out,
            &[u8::from(function.bridge.is_some()), secondary, subordinate],
        );
        put(&mut out, &window.to_le_bytes());
        let spans = function.spans.len().min(MOST_SPANS);
        put(&mut out, &[spans as u8]);
        for span in function.spans.iter().take(spans) {
            put(&mut out, &span.start.to_le_bytes());
            put(&mut out, &span.end.to_le_bytes());
        }
        let rom = function.rom.clone().unwrap_or_default();
        put(&mut out, &rom.start.to_le_bytes());
        put(&mut out, &rom.end.to_le_bytes());
    }
    out
}

/// The map [`encode`] wrote, or `None` for bytes it did not write.
pub fn decode(bytes: &[u8]) -> Option<Vec<Function>> {
    let mut at = 0usize;
    let mut take = |count: usize| -> Option<&[u8]> {
        let taken = bytes.get(at..at.checked_add(count)?)?;
        at += count;
        Some(taken)
    };
    let u64_of = |bytes: &[u8]| u64::from_le_bytes(bytes.try_into().unwrap_or([0; 8]));
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    if count > MOST_FUNCTIONS {
        return None;
    }
    let mut functions = Vec::new();
    for _ in 0..count {
        let segment = u16::from_le_bytes(take(2)?.try_into().ok()?);
        let place = take(3)?;
        let (bus, device, function) = (place[0], place[1], place[2]);
        let bridge = take(3)?;
        let (is_bridge, secondary, subordinate) = (bridge[0] != 0, bridge[1], bridge[2]);
        let window = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let spans_count = usize::from(take(1)?[0]);
        if spans_count > MOST_SPANS {
            return None;
        }
        let mut spans = Vec::new();
        for _ in 0..spans_count {
            let start = u64_of(take(8)?);
            let end = u64_of(take(8)?);
            spans.push(start..end);
        }
        let start = u64_of(take(8)?);
        let end = u64_of(take(8)?);
        functions.push(Function {
            segment,
            bus,
            device,
            function,
            bridge: is_bridge.then_some((secondary, subordinate, window)),
            spans,
            rom: (end > start).then_some(start..end),
        });
    }
    Some(functions)
}

/// Where to place an expansion ROM BAR of `bytes` for the function at
/// `segment`/`bus`/`device`/`function`: the lowest place aligned to its
/// size in the non-prefetchable window of the bridge whose secondary bus is
/// the function's, clear of every BAR of every function below that bridge
/// and of every window of the bridges below it. `None` where no bridge is
/// above the function, its window is closed or holds no room.
pub fn rom_room(map: &[Function], segment: u16, bus: u8, bytes: u64) -> Option<u64> {
    let (secondary, subordinate, window) = map.iter().find_map(|function| {
        function
            .bridge
            .filter(|&(secondary, _, _)| function.segment == segment && secondary == bus)
    })?;
    let window = bridge_window(window)?;
    let below = |function: &&Function| {
        function.segment == segment && (secondary..=subordinate).contains(&function.bus)
    };
    let mut used: Vec<Range<u64>> = Vec::new();
    for function in map.iter().filter(below) {
        used.extend(function.spans.iter().cloned());
        if let Some(window) = function.bridge.and_then(|(_, _, word)| bridge_window(word)) {
            used.push(window);
        }
    }
    place(&window, &used, bytes)
}

#[cfg(test)]
mod tests;
