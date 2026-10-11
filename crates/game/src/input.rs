//! Set-1 DOS scancode translation for controller IRQs and diagnostic scripts.

/// ASCII for an unshifted set-1 make code (0 if none).
pub fn scancode_ascii(sc: u8) -> u8 {
    const T: &[u8; 0x3A] = b"\x00\x1b1234567890-=\x08\tqwertyuiop[]\r\x00asdfghjkl;'`\x00\\zxcvbnm,./\x00*\x00 ";
    let sc = sc & 0x7F;
    T.get(sc as usize).copied().unwrap_or(0)
}
