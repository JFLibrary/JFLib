// Browsers composite the canvas in sRGB; no monitor ICC profile is exposed by the host.

/// Primary-monitor compatibility selector.
pub const ICC_MON_PRIMARY: i32 = -1;
/// The browser host cannot query the physical display's ICC profile.
pub fn monitor_icc(_window: i32) -> Option<alloc::vec::Vec<u8>> {
    None
}
/// No physical-monitor identity is exposed.
pub fn monitor_key(_window: i32) -> u64 {
    0
}
