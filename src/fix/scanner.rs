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
            result = result * 10 + (b - b'0') as u32;
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

pub fn find_message_boundary(buf: &[u8]) -> Option<usize> {
    let pattern = b"8=FIX";
    if buf.len() < pattern.len() {
        return None;
    }
    for i in 0..=buf.len() - pattern.len() {
        if buf[i..].starts_with(pattern) {
            if i > 0 {
                return Some(i);
            }
        }
    }
    None
}

pub fn extract_complete_message(buf: &[u8]) -> Option<usize> {
    let start_tag = b"8=FIX";
    if !buf.starts_with(start_tag) {
        let offset = find_message_boundary(buf)?;
        return extract_complete_message(&buf[offset..]);
    }

    let mut soh_count = 0u32;
    let mut body_len: Option<usize> = None;
    let mut pos = 0;

    while pos < buf.len() {
        let next_soh = memchr(SOH, &buf[pos..])?;
        let field_end = pos + next_soh;
        let field_data = &buf[pos..field_end];

        if let Some(eq_pos) = memchr(b'=', field_data) {
            let tag = parse_tag(&field_data[..eq_pos]);
            if let Some(tag_num) = tag {
                if tag_num == 9 {
                    let val = &field_data[eq_pos + 1..];
                    body_len = parse_usize(val);
                }
            }
        }

        soh_count += 1;
        pos = field_end + 1;

        if soh_count >= 3 && body_len.is_some() {
            let bl = body_len.unwrap();
            let body_start = pos;
            let msg_end = body_start + bl;
            if msg_end + 7 <= buf.len() {
                return Some(msg_end + 7);
            }
            return None;
        }
    }

    None
}

fn parse_usize(data: &[u8]) -> Option<usize> {
    let mut result: usize = 0;
    for &b in data {
        if b >= b'0' && b <= b'9' {
            result = result * 10 + (b - b'0') as usize;
        } else {
            return None;
        }
    }
    Some(result)
}

pub fn parse_u64(data: &[u8]) -> Option<u64> {
    let mut result: u64 = 0;
    for &b in data {
        if b >= b'0' && b <= b'9' {
            result = result * 10 + (b - b'0') as u64;
        } else {
            return None;
        }
    }
    Some(result)
}
