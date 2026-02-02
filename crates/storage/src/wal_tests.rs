// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Event, TimerId};
use std::io::Write;
use tempfile::tempdir;

fn test_event(cmd: &str) -> Event {
    Event::TimerStart {
        id: TimerId::new(format!("test:{}", cmd)),
    }
}

#[test]
fn test_open_creates_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let wal = Wal::open(&path, 0).unwrap();

    assert!(path.exists());
    assert_eq!(wal.write_seq(), 0);
    assert_eq!(wal.processed_seq(), 0);
}

#[test]
fn test_append_and_flush() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    let seq1 = wal.append(&test_event("cmd1")).unwrap();
    let seq2 = wal.append(&test_event("cmd2")).unwrap();

    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);

    wal.flush().unwrap();

    // File should now have content
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.len() > 0);
}

#[test]
fn test_next_unprocessed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    wal.append(&test_event("cmd1")).unwrap();
    wal.append(&test_event("cmd2")).unwrap();

    let entry1 = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry1.seq, 1);
    if let Event::TimerStart { id } = &entry1.event {
        assert_eq!(id, "test:cmd1");
    } else {
        panic!("Expected Timer event");
    }

    let entry2 = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry2.seq, 2);
    if let Event::TimerStart { id } = &entry2.event {
        assert_eq!(id, "test:cmd2");
    } else {
        panic!("Expected Timer event");
    }

    // No more entries
    assert!(wal.next_unprocessed().unwrap().is_none());
}

#[test]
fn test_mark_processed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    wal.append(&test_event("cmd1")).unwrap();
    wal.flush().unwrap();

    let entry = wal.next_unprocessed().unwrap().unwrap();
    wal.mark_processed(entry.seq);

    assert_eq!(wal.processed_seq(), 1);
}

#[test]
fn test_reopen_with_processed_seq() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Write some entries
    {
        let mut wal = Wal::open(&path, 0).unwrap();
        wal.append(&test_event("cmd1")).unwrap();
        wal.append(&test_event("cmd2")).unwrap();
        wal.append(&test_event("cmd3")).unwrap();
        wal.flush().unwrap();
    }

    // Reopen with processed_seq=2 (simulating recovery from snapshot)
    let mut wal = Wal::open(&path, 2).unwrap();

    // Should only get cmd3 (seq=3)
    let entry = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry.seq, 3);
    if let Event::TimerStart { id } = &entry.event {
        assert_eq!(id, "test:cmd3");
    } else {
        panic!("Expected Timer event");
    }

    assert!(wal.next_unprocessed().unwrap().is_none());
}

#[test]
fn test_entries_after() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    wal.append(&test_event("cmd1")).unwrap();
    wal.append(&test_event("cmd2")).unwrap();
    wal.append(&test_event("cmd3")).unwrap();
    wal.flush().unwrap();

    let entries = wal.entries_after(1).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 2);
    assert_eq!(entries[1].seq, 3);
}

#[test]
fn test_truncate_before() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    wal.append(&test_event("cmd1")).unwrap();
    wal.append(&test_event("cmd2")).unwrap();
    wal.append(&test_event("cmd3")).unwrap();
    wal.flush().unwrap();

    // Truncate entries before seq=2 (keep seq 2 and 3)
    wal.truncate_before(2).unwrap();

    // Check that only entries 2 and 3 remain
    let entries = wal.entries_after(0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 2);
    assert_eq!(entries[1].seq, 3);
}

/// Regression test: Shutdown events persisted in the WAL must be visible on
/// recovery so the daemon can skip them. Before the fix, the daemon's engine
/// loop would replay Event::Shutdown from the WAL and immediately exit.
#[test]
fn test_shutdown_event_survives_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Simulate: daemon processes cmd1, then receives shutdown (written to WAL)
    {
        let mut wal = Wal::open(&path, 0).unwrap();
        wal.append(&test_event("cmd1")).unwrap();
        wal.append(&Event::Shutdown).unwrap();
        wal.flush().unwrap();
    }

    // Reopen with processed_seq=1 (snapshot taken after cmd1 was processed)
    let mut wal = Wal::open(&path, 1).unwrap();

    // entries_after returns the shutdown event (seq=2 > processed_seq=1)
    let entries = wal.entries_after(1).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seq, 2);
    assert!(matches!(entries[0].event, Event::Shutdown));

    // next_unprocessed also returns it - the daemon engine loop is
    // responsible for skipping control events like Shutdown
    let entry = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry.seq, 2);
    assert!(matches!(entry.event, Event::Shutdown));

    // No more entries
    assert!(wal.next_unprocessed().unwrap().is_none());
}

#[test]
fn test_needs_flush_threshold() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();

    // Buffer is empty, should not need flush
    assert!(!wal.needs_flush());

    // Add events but not enough to trigger threshold
    for i in 0..50 {
        wal.append(&test_event(&format!("cmd{}", i))).unwrap();
    }

    // Should still not need flush (threshold is 100)
    // Note: interval might have passed, so we can't assert !needs_flush() here

    // Add more to exceed threshold
    for i in 50..101 {
        wal.append(&test_event(&format!("cmd{}", i))).unwrap();
    }

    // Now should need flush due to threshold
    assert!(wal.needs_flush());
}

#[test]
fn test_open_corrupt_wal_creates_bak_and_preserves_valid_entries() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Write valid entries then append garbage
    {
        let mut wal = Wal::open(&path, 0).unwrap();
        wal.append(&test_event("cmd1")).unwrap();
        wal.append(&test_event("cmd2")).unwrap();
        wal.flush().unwrap();
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"not-valid-json\n").unwrap();
    }

    // Open should handle corruption gracefully
    let wal = Wal::open(&path, 0).unwrap();

    // Valid entries should be preserved
    assert_eq!(wal.write_seq(), 2);

    // Corrupt file should have been rotated to .bak
    let bak = path.with_extension("bak");
    assert!(bak.exists());

    // Clean WAL should have only valid entries
    let entries = wal.entries_after(0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 1);
    assert_eq!(entries[1].seq, 2);
}

#[test]
fn test_open_corrupt_wal_rotates_bak_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Simulate 4 corrupt opens — should keep at most 3 backups
    for i in 1..=4u8 {
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&[i; 8]).unwrap();
        }

        // Open should handle corruption gracefully (fully corrupt = no valid entries)
        let wal = Wal::open(&path, 0).unwrap();
        assert_eq!(wal.write_seq(), 0);
    }

    // .bak (most recent = round 4)
    let bak1 = path.with_extension("bak");
    assert!(bak1.exists());
    assert_eq!(std::fs::read(&bak1).unwrap(), vec![4u8; 8]);

    // .bak.2 (round 3)
    let bak2 = path.with_extension("bak.2");
    assert!(bak2.exists());
    assert_eq!(std::fs::read(&bak2).unwrap(), vec![3u8; 8]);

    // .bak.3 (round 2)
    let bak3 = path.with_extension("bak.3");
    assert!(bak3.exists());
    assert_eq!(std::fs::read(&bak3).unwrap(), vec![2u8; 8]);

    // Round 1 was evicted
    let bak4 = path.with_extension("bak.4");
    assert!(!bak4.exists());
}

#[test]
fn test_entries_after_stops_at_corruption() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Write valid entries then append garbage
    {
        let mut wal = Wal::open(&path, 0).unwrap();
        wal.append(&test_event("cmd1")).unwrap();
        wal.append(&test_event("cmd2")).unwrap();
        wal.flush().unwrap();
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"corrupted-data\n").unwrap();
    }

    // Open cleans up corruption, so we corrupt after open to test entries_after
    let wal = Wal::open(&path, 0).unwrap();

    // Now append garbage directly to the underlying file
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"post-open-corruption\n").unwrap();
    }

    // entries_after should return valid entries and stop at corruption
    let entries = wal.entries_after(0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 1);
    assert_eq!(entries[1].seq, 2);
}

#[test]
fn test_next_unprocessed_skips_corrupt_entry() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();
    wal.append(&test_event("cmd1")).unwrap();
    wal.flush().unwrap();

    // Read the valid entry
    let entry = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry.seq, 1);

    // Append garbage directly to the file
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"corrupt-line\n").unwrap();
    }

    // next_unprocessed should return None (not error) on corrupt entry
    let result = wal.next_unprocessed().unwrap();
    assert!(result.is_none());

    // Append a valid entry after the corrupt one
    wal.append(&test_event("cmd2")).unwrap();
    wal.flush().unwrap();

    // Should be able to read the new valid entry (skipped past corruption)
    let entry = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry.seq, 2);
}

#[test]
fn test_open_with_binary_wal_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Write binary (non-UTF-8) data to simulate corrupt WAL
    std::fs::write(&path, b"\x80\x81\x82\xff\xfe\n").unwrap();

    // Should open successfully, treating binary data as corrupt
    let wal = Wal::open(&path, 0).unwrap();
    assert_eq!(wal.write_seq(), 0);

    // Corrupt file should have been rotated to .bak
    let bak = path.with_extension("bak");
    assert!(bak.exists());
}

#[test]
fn test_open_with_valid_entries_then_binary() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    // Write valid entries followed by binary data
    {
        let mut wal = Wal::open(&path, 0).unwrap();
        wal.append(&test_event("cmd1")).unwrap();
        wal.append(&test_event("cmd2")).unwrap();
        wal.flush().unwrap();
    }

    // Append binary garbage after the valid entries
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"\x80\x81\x82\xff\xfe\n").unwrap();
    }

    // Should open, preserve valid entries, and rotate corrupt file
    let wal = Wal::open(&path, 0).unwrap();
    assert_eq!(wal.write_seq(), 2);

    let bak = path.with_extension("bak");
    assert!(bak.exists());

    let entries = wal.entries_after(0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 1);
    assert_eq!(entries[1].seq, 2);
}

#[test]
fn test_entries_after_stops_at_binary_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();
    wal.append(&test_event("cmd1")).unwrap();
    wal.flush().unwrap();

    // Append binary garbage after valid entry
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"\x80\x81\x82\xff\xfe\n").unwrap();
    }

    // entries_after should return valid entries and stop at binary data
    let entries = wal.entries_after(0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seq, 1);
}

#[test]
fn test_next_unprocessed_handles_binary_data() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::open(&path, 0).unwrap();
    wal.append(&test_event("cmd1")).unwrap();
    wal.flush().unwrap();

    // Read the valid entry
    let entry = wal.next_unprocessed().unwrap().unwrap();
    assert_eq!(entry.seq, 1);

    // Append binary garbage
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"\x80\x81\x82\xff\xfe\n").unwrap();
    }

    // next_unprocessed should return None (not error) on binary data
    let result = wal.next_unprocessed().unwrap();
    assert!(result.is_none());
}
