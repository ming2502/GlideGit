//! Streaming parser for the `git log` records the index is built from.

/// One record per commit: id, parents, author name, author email, author
/// time, subject. Fields are NUL-separated, records end with 0x1e.
pub const FORMAT: &str = "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x1e";

const RECORD_END: u8 = 0x1e;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Raw object id (20 bytes for SHA-1, 32 for SHA-256).
    pub oid: Vec<u8>,
    /// Raw parent ids, concatenated.
    pub parents: Vec<u8>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub subject: String,
}

/// Splits a byte stream into records; chunk boundaries can fall anywhere.
#[derive(Default)]
pub struct RecordParser {
    buf: Vec<u8>,
}

impl RecordParser {
    pub fn feed(&mut self, chunk: &[u8], mut out: impl FnMut(Record)) {
        self.buf.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(i) = self.buf[start..].iter().position(|&b| b == RECORD_END) {
            if let Some(r) = parse_record(&self.buf[start..start + i]) {
                out(r);
            }
            start += i + 1;
        }
        self.buf.drain(..start);
    }
}

fn parse_record(raw: &[u8]) -> Option<Record> {
    let raw = &raw[raw.iter().position(|&b| b != b'\n')?..];
    let mut fields = raw.splitn(6, |&b| b == 0);
    let oid = hex_decode(std::str::from_utf8(fields.next()?).ok()?)?;
    let parents_hex = std::str::from_utf8(fields.next()?).ok()?;
    let mut parents = Vec::with_capacity(oid.len() * 2);
    for p in parents_hex.split_ascii_whitespace() {
        parents.extend(hex_decode(p)?);
    }
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let author_name = text(fields.next()?);
    let author_email = text(fields.next()?);
    let author_time = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
    let subject = text(fields.next()?);
    Some(Record {
        oid,
        parents,
        author_name,
        author_email,
        author_time,
        subject,
    })
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    s.as_bytes()
        .chunks(2)
        .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
        .collect()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const C: &str = "3333333333333333333333333333333333333333";

    #[test]
    fn hex_roundtrip() {
        let b = hex_decode("00ff10Ab").unwrap();
        assert_eq!(b, vec![0, 255, 16, 171]);
        assert_eq!(hex_encode(&b), "00ff10ab");
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn parses_across_chunk_boundaries() {
        let stream = format!(
            "{A}\x00{B} {C}\x00Ann\x00ann@x\x00100\x00Merge it\x1e\n{B}\x00\x00Bob\x00bob@x\x0050\x00Root\x1e\n"
        );
        let bytes = stream.as_bytes();
        // Every split point must give the same result.
        for split in 0..bytes.len() {
            let mut p = RecordParser::default();
            let mut out = Vec::new();
            p.feed(&bytes[..split], |r| out.push(r));
            p.feed(&bytes[split..], |r| out.push(r));
            assert_eq!(out.len(), 2, "split at {split}");
            assert_eq!(hex_encode(&out[0].oid), A);
            assert_eq!(out[0].parents.len(), 40);
            assert_eq!(hex_encode(&out[0].parents[20..]), C);
            assert_eq!(out[0].subject, "Merge it");
            assert_eq!(out[1].author_time, 50);
            assert!(out[1].parents.is_empty());
        }
    }
}
