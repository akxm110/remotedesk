//! Progress of files copied and pasted through the clipboard, for the remote window's title bar.
//! The file list that goes ahead of the data carries every file's size, so counting the data
//! bytes that pass gives a percentage in either direction.
use clipboard::ClipboardFile;
use hbb_common::rendezvous_proto::ConnType;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

// FILEDESCRIPTORW: nFileSizeHigh and nFileSizeLow follow 64 bytes of other fields.
const DESCRIPTOR_LEN: usize = 592;
const SIZE_OFFSET: usize = 64;
// A file list asked for again within this time is the same paste, not a new one.
const SAME_COPY: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Transfer {
    total: u64,
    done: u64,
    shown: Option<u64>,
    last: Option<Instant>,
}

lazy_static::lazy_static! {
    // (peer id, into this computer)
    static ref COPIES: Mutex<HashMap<(String, bool), Transfer>> = Default::default();
}

/// A clipboard message the peer sent: data for a paste on this computer.
pub fn from_peer(peer_id: &str, clip: &ClipboardFile) {
    track(peer_id, true, clip);
}

/// A clipboard message going to the peer: data for a paste on the remote computer.
pub fn to_peer(peer_id: &str, clip: &ClipboardFile) {
    track(peer_id, false, clip);
}

fn track(peer_id: &str, inward: bool, clip: &ClipboardFile) {
    let report = {
        let Ok(mut copies) = COPIES.lock() else {
            return;
        };
        let copy = copies.entry((peer_id.to_owned(), inward)).or_default();
        match clip {
            ClipboardFile::FormatDataResponse { format_data, .. } => {
                let Some(total) = list_total(format_data) else {
                    return;
                };
                let same = total == copy.total
                    && copy.last.map_or(false, |t| t.elapsed() < SAME_COPY);
                if !same {
                    *copy = Transfer {
                        total,
                        ..Default::default()
                    };
                }
                None
            }
            ClipboardFile::FileContentsResponse { requested_data, .. } if copy.total > 0 => {
                copy.done += requested_data.len() as u64;
                copy.last = Some(Instant::now());
                let pct = (copy.done.min(copy.total) * 100 / copy.total).min(100);
                (copy.shown != Some(pct)).then(|| {
                    copy.shown = Some(pct);
                    pct
                })
            }
            _ => None,
        }
    };
    if let Some(pct) = report {
        if let Some(s) =
            crate::flutter::sessions::get_session_by_peer_id(peer_id.to_owned(), ConnType::DEFAULT_CONN)
        {
            let dir = if inward { "in" } else { "out" };
            let pct = pct.to_string();
            s.ui_handler
                .push_event("clip_progress", &[("dir", dir), ("pct", pct.as_str())], &[]);
        }
    }
}

/// The total size in a FILEGROUPDESCRIPTORW, or None for other clipboard data.
fn list_total(data: &[u8]) -> Option<u64> {
    let count = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
    if count == 0 || data.len() != 4 + count.checked_mul(DESCRIPTOR_LEN)? {
        return None;
    }
    let total = data[4..]
        .chunks_exact(DESCRIPTOR_LEN)
        .map(|d| {
            let word = |at: usize| u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]);
            (word(SIZE_OFFSET) as u64) << 32 | word(SIZE_OFFSET + 4) as u64
        })
        .sum();
    (total > 0).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_list_sizes_add_up() {
        let mut data = 2u32.to_le_bytes().to_vec();
        for size in [1u64 << 32 | 5, 7] {
            let mut d = vec![0u8; DESCRIPTOR_LEN];
            d[SIZE_OFFSET..SIZE_OFFSET + 4].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
            d[SIZE_OFFSET + 4..SIZE_OFFSET + 8].copy_from_slice(&(size as u32).to_le_bytes());
            data.extend(d);
        }
        assert_eq!(list_total(&data), Some((1u64 << 32) + 12));
        assert_eq!(list_total(b"hello"), None);
    }
}
