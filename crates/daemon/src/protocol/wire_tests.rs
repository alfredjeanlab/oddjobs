// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Wire format tests: length-prefix framing and JSON encoding.

use super::*;

#[test]
fn encode_returns_json_without_length_prefix() {
    let response = Response::Ok;
    let encoded = encode(&response).expect("encode failed");

    // encode() returns raw JSON, no length prefix
    let json_str = std::str::from_utf8(&encoded).expect("should be valid UTF-8");
    assert!(json_str.starts_with('{'), "should be JSON object: {}", json_str);
}

#[tokio::test]
async fn read_write_message_roundtrip() {
    let original = b"hello world";

    let mut buffer = Vec::new();
    write_message(&mut buffer, original).await.expect("write failed");

    // write_message adds 4-byte length prefix
    assert_eq!(buffer.len(), 4 + original.len());

    let mut cursor = std::io::Cursor::new(buffer);
    let read_back = read_message(&mut cursor).await.expect("read failed");

    assert_eq!(read_back, original);
}

#[tokio::test]
async fn write_message_adds_length_prefix() {
    let data = b"test data";

    let mut buffer = Vec::new();
    write_message(&mut buffer, data).await.expect("write failed");

    // First 4 bytes are the length prefix
    let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

    // Length should match the data size
    assert_eq!(len, data.len());
    assert_eq!(&buffer[4..], data);
}
