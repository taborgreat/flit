//! The only place in flit-core that reads a clock. Time is information,
//! never order: what comes out of here goes into `stamped_at` (signed, in
//! the act) or `received_at` (local sidecar), both display only.

/// Unix time in milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
