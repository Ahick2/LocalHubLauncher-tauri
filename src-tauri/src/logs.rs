use crate::model::now_ms;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

pub const MAX_LOG_LINES: usize = 4000;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_ITEM_BYTES: usize = 1024 * 1024;
const MAX_COMBINED_BYTES: usize = 4 * 1024 * 1024;
const MAX_ALL_ITEMS_BYTES: usize = 16 * 1024 * 1024;
const MAX_PENDING_LINES: usize = 512;
const MAX_PENDING_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub sequence: u64,
    pub item_id: Uuid,
    pub item_name: String,
    pub text: String,
    pub stream: LogStream,
    pub timestamp: u64,
}

impl LogRecord {
    fn bytes(&self) -> usize {
        self.text.len() + self.item_name.len() + 64
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogTab {
    pub id: Uuid,
    pub name: String,
    pub count: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBatch {
    pub entries: Vec<LogRecord>,
    pub dropped: u64,
}

#[derive(Serialize)]
pub struct LogPage {
    pub entries: Vec<LogRecord>,
    pub cursor: u64,
}

#[derive(Default)]
struct Ring {
    name: String,
    records: VecDeque<LogRecord>,
    bytes: usize,
}

impl Ring {
    fn push(&mut self, record: LogRecord, byte_limit: usize) {
        self.bytes += record.bytes();
        self.records.push_back(record);
        while self.records.len() > MAX_LOG_LINES || self.bytes > byte_limit {
            self.pop();
        }
    }

    fn pop(&mut self) -> usize {
        if let Some(record) = self.records.pop_front() {
            let bytes = record.bytes();
            self.bytes -= bytes;
            bytes
        } else {
            0
        }
    }
}

#[derive(Default)]
struct Inner {
    next_sequence: u64,
    combined: Ring,
    items: HashMap<Uuid, Ring>,
    item_bytes: usize,
    pending: VecDeque<LogRecord>,
    pending_bytes: usize,
    dropped: u64,
}

#[derive(Default)]
pub struct LogStore {
    inner: Mutex<Inner>,
}

impl LogStore {
    pub fn ensure(&self, id: Uuid, name: &str) {
        self.inner.lock().items.entry(id).or_default().name = name.to_owned();
    }

    pub fn append(&self, id: Uuid, name: &str, text: impl Into<String>, stream: LogStream) {
        let text = clip_line(text.into());
        let mut inner = self.inner.lock();
        inner.next_sequence += 1;
        let record = LogRecord {
            sequence: inner.next_sequence,
            item_id: id,
            item_name: name.to_owned(),
            text,
            stream,
            timestamp: now_ms(),
        };
        let item = inner.items.entry(id).or_default();
        item.name = name.to_owned();
        let before = item.bytes;
        item.push(record.clone(), MAX_ITEM_BYTES);
        let after = item.bytes;
        inner.item_bytes = inner.item_bytes + after - before;
        while inner.item_bytes > MAX_ALL_ITEMS_BYTES {
            let oldest = inner
                .items
                .iter()
                .filter_map(|(id, ring)| ring.records.front().map(|r| (*id, r.sequence)))
                .min_by_key(|(_, sequence)| *sequence)
                .map(|(id, _)| id);
            let Some(oldest) = oldest else {
                break;
            };
            let removed = inner.items.get_mut(&oldest).unwrap().pop();
            inner.item_bytes -= removed;
        }
        inner.combined.push(record.clone(), MAX_COMBINED_BYTES);
        inner.pending_bytes += record.bytes();
        inner.pending.push_back(record);
        while inner.pending.len() > MAX_PENDING_LINES || inner.pending_bytes > MAX_PENDING_BYTES {
            if let Some(record) = inner.pending.pop_front() {
                inner.pending_bytes -= record.bytes();
                inner.dropped += 1;
            }
        }
    }

    #[cfg(test)]
    pub fn read(&self, id: Option<Uuid>) -> Vec<LogRecord> {
        self.page(id).entries
    }

    pub fn page(&self, id: Option<Uuid>) -> LogPage {
        let inner = self.inner.lock();
        let ring = match id {
            Some(id) => inner.items.get(&id),
            None => Some(&inner.combined),
        };
        LogPage {
            entries: ring
                .map(|ring| ring.records.iter().cloned().collect())
                .unwrap_or_default(),
            cursor: inner.next_sequence,
        }
    }

    pub fn tabs(&self) -> Vec<LogTab> {
        let inner = self.inner.lock();
        let mut tabs: Vec<_> = inner
            .items
            .iter()
            .map(|(id, ring)| LogTab {
                id: *id,
                name: ring.name.clone(),
                count: ring.records.len(),
            })
            .collect();
        tabs.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        tabs
    }

    pub fn clear(&self, id: Option<Uuid>) {
        let mut inner = self.inner.lock();
        if let Some(id) = id {
            if let Some(ring) = inner.items.get_mut(&id) {
                let bytes = ring.bytes;
                ring.records.clear();
                ring.bytes = 0;
                inner.item_bytes -= bytes;
            }
            inner.pending.retain(|record| record.item_id != id);
        } else {
            inner.combined = Ring::default();
            inner.pending.clear();
            inner.dropped = 0;
        }
        inner.pending_bytes = inner.pending.iter().map(LogRecord::bytes).sum();
    }

    pub fn remove(&self, id: Uuid) {
        let mut inner = self.inner.lock();
        if let Some(ring) = inner.items.remove(&id) {
            inner.item_bytes -= ring.bytes;
        }
        inner.pending.retain(|record| record.item_id != id);
        inner.pending_bytes = inner.pending.iter().map(LogRecord::bytes).sum();
    }

    pub fn drain_batch(&self) -> Option<LogBatch> {
        let mut inner = self.inner.lock();
        if inner.pending.is_empty() && inner.dropped == 0 {
            return None;
        }
        let entries = inner.pending.drain(..).collect();
        inner.pending_bytes = 0;
        Some(LogBatch {
            entries,
            dropped: std::mem::take(&mut inner.dropped),
        })
    }
}

fn clip_line(mut text: String) -> String {
    if text.len() > MAX_LINE_BYTES {
        let mut end = MAX_LINE_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(" … [单行日志已截断]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_flood_has_bounded_history_and_pending_events() {
        let store = LogStore::default();
        let id = Uuid::new_v4();
        for index in 0..10_000 {
            store.append(
                id,
                "demo",
                format!("{index}: {}", "x".repeat(200)),
                LogStream::Stdout,
            );
        }
        let inner = store.inner.lock();
        assert!(inner.combined.records.len() <= MAX_LOG_LINES);
        assert!(inner.combined.bytes <= MAX_COMBINED_BYTES);
        assert!(inner.items[&id].bytes <= MAX_ITEM_BYTES);
        assert!(inner.pending.len() <= MAX_PENDING_LINES);
        assert!(inner.pending_bytes <= MAX_PENDING_BYTES);
        assert!(inner.dropped > 0);
        assert!(inner
            .combined
            .records
            .back()
            .unwrap()
            .text
            .starts_with("9999:"));
    }

    #[test]
    fn clearing_a_tab_does_not_replay_queued_lines() {
        let store = LogStore::default();
        let id = Uuid::new_v4();
        store.append(id, "demo", "old", LogStream::Stdout);
        store.clear(Some(id));
        assert!(store.read(Some(id)).is_empty());
        assert!(store.drain_batch().is_none());
        assert_eq!(store.read(None).len(), 1);
    }
}
