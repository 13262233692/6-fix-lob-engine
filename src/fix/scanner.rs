const SOH: u8 = 0x01;

pub struct FixScanner<'a> {
    data: &'a [u8],
    pos: usize,
}

#[derive(Clone, Copy)]
pub struct FixField<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

#[derive(Default)]
pub struct FixMessage<'a> {
    pub msg_type: Option<&'a [u8]>,
    pub symbol: Option<&'a [u8]>,
    pub price: Option<&'a [u8]>,
    pub side: Option<&'a [u8]>,
    pub order_qty: Option<&'a [u8]>,
    pub clord_id: Option<&'a [u8]>,
    pub sender: Option<&'a [u8]>,
    pub target: Option<&'a [u8]>,
}

impl<'a> FixMessage<'a> {
    pub fn scan(data: &'a [u8]) -> FixMessage<'a> {
        let mut scanner = FixScanner::new(data);
        let mut msg = FixMessage::default();
        while let Some(field) = scanner.next_field() {
            match field.tag {
                35 => msg.msg_type = Some(field.value),
                55 => msg.symbol = Some(field.value),
                44 => msg.price = Some(field.value),
                54 => msg.side = Some(field.value),
                38 => msg.order_qty = Some(field.value),
                11 => msg.clord_id = Some(field.value),
                49 => msg.sender = Some(field.value),
                56 => msg.target = Some(field.value),
                _ => {}
            }
        }
        msg
    }
}

impl<'a> FixScanner<'a> {
    #[inline(always)]
    pub fn new(data: &'a [u8]) -> Self {
        FixScanner { data, pos: 0 }
    }

    #[inline(always)]
    pub fn next_field(&mut self) -> Option<FixField<'a>> {
        if self.pos >= self.data.len() {
            return None;
        }
        let remaining = &self.data[self.pos..];

        let eq_pos = memchr(b'=', remaining)?;
        let tag_end = self.pos + eq_pos;
        let tag = parse_tag(&self.data[self.pos..tag_end])?;

        let value_start = tag_end + 1;
        if value_start > self.data.len() {
            return None;
        }
        let soh_pos = memchr(SOH, &self.data[value_start..])?;
        let value_end = value_start + soh_pos;

        let value = &self.data[value_start..value_end];
        self.pos = value_end + 1;

        Some(FixField { tag, value })
    }
}

#[inline(always)]
fn parse_tag(data: &[u8]) -> Option<u32> {
    let mut result: u32 = 0;
    for &b in data {
        if b >= b'0' && b <= b'9' {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u32)?;
        } else {
            return None;
        }
    }
    Some(result)
}

#[inline(always)]
fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

pub fn parse_u64(data: &[u8]) -> Option<u64> {
    let mut result: u64 = 0;
    for &b in data {
        if b >= b'0' && b <= b'9' {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u64)?;
        } else {
            return None;
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tag_overflow() {
        assert_eq!(parse_tag(b"4294967295"), Some(4294967295u32));
        assert_eq!(parse_tag(b"4294967296"), None);
    }

    #[test]
    fn test_parse_u64_overflow() {
        assert_eq!(parse_u64(b"18446744073709551615"), Some(18446744073709551615u64));
        assert_eq!(parse_u64(b"18446744073709551616"), None);
    }

    #[test]
    fn test_scanner_boundary() {
        let data = b"35=D\x0155=AAPL\x0144=100.50\x01";
        let mut scanner = FixScanner::new(data);
        let f1 = scanner.next_field().unwrap();
        assert_eq!(f1.tag, 35);
        assert_eq!(f1.value, b"D");
        let f2 = scanner.next_field().unwrap();
        assert_eq!(f2.tag, 55);
        assert_eq!(f2.value, b"AAPL");
        let f3 = scanner.next_field().unwrap();
        assert_eq!(f3.tag, 44);
        assert_eq!(f3.value, b"100.50");
        assert!(scanner.next_field().is_none());
    }

    #[test]
    fn test_scanner_truncated_no_soh() {
        let data = b"35=D\x0155=AAPL\x0144=100.50";
        let mut scanner = FixScanner::new(data);
        let f1 = scanner.next_field().unwrap();
        assert_eq!(f1.tag, 35);
        let f2 = scanner.next_field().unwrap();
        assert_eq!(f2.tag, 55);
        assert!(scanner.next_field().is_none());
    }

    #[test]
    fn test_scanner_truncated_no_value() {
        let data = b"35=";
        let mut scanner = FixScanner::new(data);
        assert!(scanner.next_field().is_none());
    }
}
