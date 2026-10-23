//! Link-level chunking (protocol.md §4).

pub const HID_REPORT_IN: u8 = 0xA1;
pub const HID_REPORT_OUT: u8 = 0xA2;
pub const HID_PAYLOAD: usize = 63;

/// Host → device HID output reports: report ID + 63 bytes, the last one zero padded.
pub fn hid_reports(data: &[u8]) -> Vec<[u8; HID_PAYLOAD + 1]> {
    data.chunks(HID_PAYLOAD)
        .map(|c| {
            let mut r = [0u8; HID_PAYLOAD + 1];
            r[0] = HID_REPORT_OUT;
            r[1..1 + c.len()].copy_from_slice(c);
            r
        })
        .collect()
}

/// STLINK VCP workaround: a write whose length is a multiple of 8 is split so the last write is
/// one byte (the caller waits ≥ 10 ms between the two writes).
pub fn stlink_split(data: &[u8]) -> Vec<&[u8]> {
    if data.len() > 1 && data.len().is_multiple_of(8) {
        vec![&data[..data.len() - 1], &data[data.len() - 1..]]
    } else {
        vec![data]
    }
}

/// Gap between the two halves of an STLINK split write.
pub const STLINK_SPLIT_GAP: std::time::Duration = std::time::Duration::from_millis(10);

/// Default serial discovery filter (protocol.md §4.1). `None` PID = any.
pub const SERIAL_FILTER: &[(u16, Option<u16>, &str)] = &[
    (0x0483, Some(0x374B), "STLINK"),
    (0x0483, Some(0x374E), "STLINK"),
    (0x0483, Some(0x374F), "STLINK"),
    (0x0483, Some(0x3753), "STLINK"),
    (0x0483, Some(0x3754), "STLINK"),
    (0x0403, Some(0x6001), "FTDI"),
    (0x0403, Some(0x6010), "FTDI"),
    (0x0403, Some(0x6011), "FTDI"),
    (0x0403, Some(0x6014), "FTDI"),
    (0x0403, Some(0x6015), "FTDI"),
    (0x2FE3, None, "Zephyr CDC-ACM"),
];

pub const HID_VID: u16 = 0x2FE3;
pub const HID_INTERFACE: i32 = 1;
pub const BLE_UUID: &str = "8683b8f3-0000-1000-8000-00805f9b34fb";
pub const DEFAULT_BAUD: u32 = 1_000_000;
pub const DEFAULT_HUB_PORT: u16 = 9000;
