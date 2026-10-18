/// Build the sync entry key: `reviews/<hlc-prefixed, zero-padded>-<device>-<uuidv4-prefix>`.
/// Zero-padded millisecond timestamp keeps lexicographic key order chronological.
pub fn entry_key(i: u32, device: &str) -> String {
    // synthetic but realistic HLC-millis timestamps, strictly increasing
    let ms = 1_700_000_000_000u64 + i as u64 * 60_000;
    let p = uuid_prefix(i);
    format!("reviews/{ms:013}-{device}-{p}")
}

fn uuid_prefix(i: u32) -> String {
    format!("{:08x}", i.wrapping_mul(0x9e3779b9) ^ 0xa5a5a5a5)
}

/// ~300-byte JSON review event matching our sync batch shape.
pub fn review_event_json(i: u32, device: &str) -> String {
    let json = serde_json::json!({
        "uuid": format!("{}-0000-4000-8000-{:012x}", uuid_prefix(i), i),
        "card_id": "athenaze-i-0001",
        "timestamp": format!("2026-10-17T10:{:02}:{:02}Z", (i / 60) % 60, i % 60),
        "rating": (i % 4) + 1,
        "elapsed_days": 2.0,
        "scheduled_days": (i % 7) as f64 + 1.0,
        "source": "flashcard",
        "device_id": device,
        "hlc": format!("{:016x}:{:04x}:{}", 1_700_000_000_000u64 + i as u64 * 60_000, i % 16, device),
        "notes": "padding padding padding padding to reach the ~300 byte payload size expected",
    });
    let s = json.to_string();
    debug_assert!((250..=400).contains(&s.len()), "payload {} bytes", s.len());
    s
}
