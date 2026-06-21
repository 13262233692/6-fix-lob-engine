use std::io::{self, Read};

const SOH: u8 = 0x01;
const MAX_FIX_BODY_LEN: usize = 65535;
const FIX_HEADER_MIN: usize = 14;
const FIX_TRAILER_LEN: usize = 7;
const TAG_9_MAX_DIGITS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Complete(usize),
    Incomplete,
    Invalid,
}

pub struct LookaheadRingBuffer {
    buf: Vec<u8>,
    head: usize,
    tail: usize,
    len: usize,
    capacity: usize,
    mark: usize,
}

impl LookaheadRingBuffer {
    pub fn new(capacity: usize) -> Self {
        LookaheadRingBuffer {
            buf: vec![0u8; capacity],
            head: 0,
            tail: 0,
            len: 0,
            capacity,
            mark: 0,
        }
    }

    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn available(&self) -> usize {
        self.capacity - self.len
    }

    pub fn write_from<R: Read>(&mut self, reader: &mut R, max_bytes: usize) -> io::Result<usize> {
        let to_read = max_bytes.min(self.available());
        if to_read == 0 {
            return Ok(0);
        }

        let mut tmp = vec![0u8; to_read];
        let n = reader.read(&mut tmp)?;
        if n > 0 {
            self.extend_from_slice(&tmp[..n]);
        }
        Ok(n)
    }

    pub fn extend_from_slice(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.available());
        if n == 0 {
            return 0;
        }

        for i in 0..n {
            let idx = (self.tail + i) % self.capacity;
            self.buf[idx] = data[i];
        }
        self.tail = (self.tail + n) % self.capacity;
        self.len += n;
        n
    }

    #[inline(always)]
    fn idx(&self, offset: usize) -> usize {
        (self.head + offset) % self.capacity
    }

    #[inline(always)]
    pub fn peek(&self, offset: usize) -> Option<u8> {
        if offset >= self.len {
            return None;
        }
        Some(self.buf[self.idx(offset)])
    }

    pub fn peek_slice(&self, offset: usize, length: usize) -> Option<Vec<u8>> {
        if length == 0 {
            return Some(Vec::new());
        }
        if offset.checked_add(length).is_none() || offset + length > self.len {
            return None;
        }
        let mut result = Vec::with_capacity(length);
        for i in 0..length {
            result.push(self.buf[self.idx(offset + i)]);
        }
        Some(result)
    }

    pub fn memchr(&self, needle: u8, start: usize) -> Option<usize> {
        if start >= self.len {
            return None;
        }
        for i in start..self.len {
            if self.buf[self.idx(i)] == needle {
                return Some(i);
            }
        }
        None
    }

    pub fn starts_with(&self, pattern: &[u8], start: usize) -> bool {
        if start.checked_add(pattern.len()).is_none() || start + pattern.len() > self.len {
            return false;
        }
        for (i, &b) in pattern.iter().enumerate() {
            if self.buf[self.idx(start + i)] != b {
                return false;
            }
        }
        true
    }

    pub fn mark(&mut self) {
        self.mark = self.head;
    }

    pub fn reset_to_mark(&mut self) {
        self.head = self.mark;
        self.len = (self.tail + self.capacity - self.head) % self.capacity;
    }

    pub fn consume(&mut self, n: usize) -> bool {
        if n > self.len {
            return false;
        }
        self.head = (self.head + n) % self.capacity;
        self.len -= n;
        true
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
        self.mark = 0;
    }

    pub fn compact(&mut self) {
        if self.is_empty() {
            self.head = 0;
            self.tail = 0;
            return;
        }
        let mut tmp = vec![0u8; self.len];
        for i in 0..self.len {
            tmp[i] = self.buf[self.idx(i)];
        }
        self.buf[..self.len].copy_from_slice(&tmp);
        self.head = 0;
        self.tail = self.len;
    }

    pub fn find_next_message_start(&self, start: usize) -> Option<usize> {
        let pattern = b"8=FIX";
        if start + pattern.len() > self.len {
            return None;
        }
        let mut i = start;
        while i + pattern.len() <= self.len {
            if self.starts_with(pattern, i) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    pub fn extract_complete_message(&mut self) -> ScanStatus {
        if self.len < FIX_HEADER_MIN {
            return ScanStatus::Incomplete;
        }

        let _msg_start = match self.find_next_message_start(0) {
            Some(pos) if pos == 0 => 0,
            Some(pos) if pos > 0 => {
                self.consume(pos);
                if self.len < FIX_HEADER_MIN {
                    return ScanStatus::Incomplete;
                }
                0
            }
            Some(_) => {
                self.clear();
                return ScanStatus::Invalid;
            }
            None => {
                if self.len > 1024 * 1024 {
                    self.clear();
                    return ScanStatus::Invalid;
                }
                return ScanStatus::Incomplete;
            }
        };

        let mut scan_pos = 0;
        let mut soh_count = 0u32;
        let mut body_len: Option<usize> = None;

        while scan_pos < self.len {
            let soh_pos = match self.memchr(SOH, scan_pos) {
                Some(pos) => pos,
                None => return ScanStatus::Incomplete,
            };

            soh_count += 1;

            if body_len.is_none() {
                let eq_pos = self.memchr(b'=', scan_pos);
                if let Some(eq) = eq_pos {
                    if eq < soh_pos {
                        let tag_digits = eq - scan_pos;
                        if tag_digits > 0 && tag_digits <= 5 {
                            if let Some(tag_val) = self.parse_tag_safe(scan_pos, eq) {
                                if tag_val == 9 {
                                    let val_start = eq + 1;
                                    let val_len = soh_pos - val_start;
                                    if val_len == 0 || val_len > TAG_9_MAX_DIGITS {
                                        self.skip_to_next_message(soh_pos + 1);
                                        return ScanStatus::Invalid;
                                    }
                                    match self.parse_usize_safe(val_start, soh_pos) {
                                        Some(bl) if bl > MAX_FIX_BODY_LEN => {
                                            self.skip_to_next_message(soh_pos + 1);
                                            return ScanStatus::Invalid;
                                        }
                                        Some(bl) => body_len = Some(bl),
                                        None => {
                                            self.skip_to_next_message(soh_pos + 1);
                                            return ScanStatus::Invalid;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            scan_pos = soh_pos + 1;

            if soh_count >= 3 && body_len.is_some() {
                let bl = body_len.unwrap();
                let body_start = scan_pos;

                let msg_end = match body_start.checked_add(bl) {
                    Some(e) => e,
                    None => {
                        self.skip_to_next_message(scan_pos);
                        return ScanStatus::Invalid;
                    }
                };

                let total_needed = match msg_end.checked_add(FIX_TRAILER_LEN) {
                    Some(t) => t,
                    None => {
                        self.skip_to_next_message(scan_pos);
                        return ScanStatus::Invalid;
                    }
                };

                if total_needed > self.capacity {
                    self.skip_to_next_message(scan_pos);
                    return ScanStatus::Invalid;
                }

                if total_needed <= self.len {
                    return ScanStatus::Complete(total_needed);
                } else {
                    return ScanStatus::Incomplete;
                }
            }
        }

        ScanStatus::Incomplete
    }

    fn skip_to_next_message(&mut self, search_from: usize) {
        if search_from >= self.len {
            self.clear();
            return;
        }
        match self.find_next_message_start(search_from) {
            Some(next_start) => {
                self.consume(next_start);
            }
            None => {
                self.clear();
            }
        }
    }

    fn parse_tag_safe(&self, start: usize, eq_pos: usize) -> Option<u32> {
        if eq_pos <= start || eq_pos > self.len {
            return None;
        }
        let mut result: u32 = 0;
        for i in start..eq_pos {
            let b = self.buf[self.idx(i)];
            if b < b'0' || b > b'9' {
                return None;
            }
            result = match result.checked_mul(10) {
                Some(r) => r,
                None => return None,
            };
            result = match result.checked_add((b - b'0') as u32) {
                Some(r) => r,
                None => return None,
            };
        }
        Some(result)
    }

    fn parse_usize_safe(&self, start: usize, end: usize) -> Option<usize> {
        if end <= start || end > self.len {
            return None;
        }
        let mut result: usize = 0;
        for i in start..end {
            let b = self.buf[self.idx(i)];
            if b < b'0' || b > b'9' {
                return None;
            }
            result = match result.checked_mul(10) {
                Some(r) => r,
                None => return None,
            };
            result = match result.checked_add((b - b'0') as usize) {
                Some(r) => r,
                None => return None,
            };
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fix_message(_seq: u32, body: &[u8]) -> Vec<u8> {
        let header = format!("8=FIX.4.4\x019={}\x0135=D\x01", body.len());
        let mut msg = Vec::new();
        msg.extend_from_slice(header.as_bytes());
        msg.extend_from_slice(body);
        let mut checksum: u8 = 0;
        for &b in &msg {
            checksum = checksum.wrapping_add(b);
        }
        let trailer = format!("10={:03}\x01", checksum);
        msg.extend_from_slice(trailer.as_bytes());
        msg
    }

    #[test]
    fn test_ringbuffer_basic() {
        let mut rb = LookaheadRingBuffer::new(1024);
        assert!(rb.is_empty());
        let data = b"8=FIX.4.4\x019=5\x0135=D\x01";
        rb.extend_from_slice(data);
        assert_eq!(rb.len(), 19);
        assert_eq!(rb.peek(0), Some(b'8'));
        assert_eq!(rb.peek(18), Some(0x01));
        assert_eq!(rb.peek(19), None);
    }

    #[test]
    fn test_ringbuffer_wrap_around() {
        let mut rb = LookaheadRingBuffer::new(16);
        rb.extend_from_slice(b"0123456789");
        rb.consume(6);
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.peek(0), Some(b'6'));
        rb.extend_from_slice(b"ABCDEFGHIJ");
        assert_eq!(rb.len(), 14);
        assert_eq!(rb.peek(0), Some(b'6'));
        assert_eq!(rb.peek(13), Some(b'J'));
    }

    #[test]
    fn test_extract_complete_message_valid() {
        let body = b"11=ORD001\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x01";
        let msg = make_fix_message(1, body);
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&msg);
        let status = rb.extract_complete_message();
        match status {
            ScanStatus::Complete(len) => assert_eq!(len, msg.len()),
            _ => panic!("Expected complete message"),
        }
    }

    #[test]
    fn test_extract_complete_message_truncated() {
        let body = b"11=ORD001\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x01";
        let msg = make_fix_message(1, body);
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&msg[..msg.len() / 2]);
        let status = rb.extract_complete_message();
        assert_eq!(status, ScanStatus::Incomplete);
    }

    #[test]
    fn test_extract_complete_message_huge_body_len() {
        let mut msg = Vec::new();
        msg.extend_from_slice(b"8=FIX.4.4\x019=9999999\x0135=D\x01");
        msg.extend_from_slice(&[0u8; 100]);
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&msg);
        let status = rb.extract_complete_message();
        assert_eq!(status, ScanStatus::Invalid);
    }

    #[test]
    fn test_extract_complete_message_malformed_tag9() {
        let mut msg = Vec::new();
        msg.extend_from_slice(b"8=FIX.4.4\x019=ABC\x0135=D\x01");
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&msg);
        let status = rb.extract_complete_message();
        assert_eq!(status, ScanStatus::Invalid);
    }

    #[test]
    fn test_extract_complete_message_overflow_body_len() {
        let mut msg = Vec::new();
        msg.extend_from_slice(b"8=FIX.4.4\x019=4294967295\x0135=D\x01");
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&msg);
        let status = rb.extract_complete_message();
        assert_eq!(status, ScanStatus::Invalid);
    }

    #[test]
    fn test_extract_complete_message_multi_msg() {
        let body1 = b"11=ORD001\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x01";
        let body2 = b"11=ORD002\x0154=2\x0155=GOOG\x0144=1500.25\x0138=200\x01";
        let msg1 = make_fix_message(1, body1);
        let msg2 = make_fix_message(2, body2);
        let mut combined = Vec::new();
        combined.extend_from_slice(&msg1);
        combined.extend_from_slice(&msg2);

        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&combined);

        let status1 = rb.extract_complete_message();
        match status1 {
            ScanStatus::Complete(len) => {
                assert_eq!(len, msg1.len());
                rb.consume(len);
            }
            _ => panic!("Expected first complete message"),
        }

        let status2 = rb.extract_complete_message();
        match status2 {
            ScanStatus::Complete(len) => assert_eq!(len, msg2.len()),
            _ => panic!("Expected second complete message"),
        }
    }

    #[test]
    fn test_extract_complete_message_mtu_boundary() {
        let body = b"11=ORD001\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x01";
        let msg = make_fix_message(1, body);
        let split_at = msg.len() / 2;
        let part1 = &msg[..split_at];
        let part2 = &msg[split_at..];

        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(part1);
        assert_eq!(rb.extract_complete_message(), ScanStatus::Incomplete);
        rb.extend_from_slice(part2);
        match rb.extract_complete_message() {
            ScanStatus::Complete(len) => assert_eq!(len, msg.len()),
            _ => panic!("Expected complete message after reassembly"),
        }
    }

    #[test]
    fn test_extract_complete_message_garbage_prefix() {
        let body = b"11=ORD001\x0154=1\x0155=AAPL\x0144=100.50\x0138=100\x01";
        let msg = make_fix_message(1, body);
        let mut combined = Vec::new();
        combined.extend_from_slice(b"\x00\x01\xFFGARBAGEJUNK\x01");
        combined.extend_from_slice(&msg);

        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(&combined);
        match rb.extract_complete_message() {
            ScanStatus::Complete(len) => assert_eq!(len, msg.len()),
            other => panic!("Expected complete message after garbage skip, got {:?}", other),
        }
    }

    #[test]
    fn test_starts_with_ring_wrapped() {
        let mut rb = LookaheadRingBuffer::new(16);
        rb.extend_from_slice(b"XXXXXXXXXX");
        rb.consume(10);
        assert!(rb.is_empty());
        rb.extend_from_slice(b"8=FIX.4.4");
        assert!(rb.starts_with(b"8=FIX.4.4", 0));
        assert!(!rb.starts_with(b"8=FIX.4.5", 0));

        let mut rb2 = LookaheadRingBuffer::new(16);
        rb2.extend_from_slice(b"0123456789ABCDEF");
        rb2.consume(10);
        assert_eq!(rb2.len(), 6);
        rb2.extend_from_slice(b"XYZ");
        assert!(rb2.starts_with(b"ABCDEFXYZ", 0));
    }

    #[test]
    fn test_extract_complete_message_integer_overflow_attack() {
        let mut rb = LookaheadRingBuffer::new(4096);
        rb.extend_from_slice(b"8=FIX.4.4\x019=18446744073709551615\x0135=D\x01");
        let status = rb.extract_complete_message();
        assert_eq!(status, ScanStatus::Invalid);
        assert!(rb.is_empty() || rb.len() < 50);
    }
}
