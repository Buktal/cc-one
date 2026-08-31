//! Device id 原语：什么是合法 deviceId、首代怎么生成、无用户命名时的默认
//! 展示名。registry 的知识，归 devices 域；`config` 的 bootstrap
//! （`ConfigStore::load_at`）在首次运行 / 配置损坏回退时经
//! `crate::devices` 的 re-export 调用首代。

use rand::Rng;

use crate::config::Paths;

use super::iter_data_device_ids;

/// A valid deviceId is 12 lowercase hex chars (48-bit short id).
pub fn is_valid_device_id(id: &str) -> bool {
    id.len() == 12
        && id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (!c.is_ascii_alphabetic() || c.is_ascii_lowercase()))
}

/// Generate a 12-hex deviceId (48 bits), retrying if it collides with an
/// existing device dir in `repo/data/` (collision check). The collision set is
/// [`iter_data_device_ids`] — the same "walk `repo/data/` for valid device
/// dirs" loop, not a second private copy; a read error degrades to "no known
/// ids" (the check is best-effort, first generation must not fail on it).
///
/// Consumed by `config`'s bootstrap (`ConfigStore::load_at`) on first run /
/// after a corrupt config fallback.
pub(crate) fn generate_device_id(paths: &Paths) -> String {
    let existing = iter_data_device_ids(paths).unwrap_or_default();
    let mut rng = rand::thread_rng();
    for _ in 0..8 {
        let bytes: [u8; 6] = rng.gen();
        let id = hex_encode(&bytes);
        if !existing.iter().any(|e| e == &id) {
            return id;
        }
    }
    // Astronomically unlikely (8 × 2^-48); fall through with the last candidate.
    let bytes: [u8; 6] = rng.gen();
    hex_encode(&bytes)
}

/// The default display name derived from the id (`Device-<first6>`): what a
/// device with no user-chosen name shows — discover (`db::store_devices`), the
/// registry read ([`super::read_all_device_artifacts`]) and the bootstrap all
/// fall back to it, never to a raw id.
pub fn default_display_name(device_id: &str) -> String {
    let prefix = &device_id[..6.min(device_id.len())];
    format!("Device-{prefix}")
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
