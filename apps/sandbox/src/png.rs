//! Screenshots as PNG (ADR 0085): 8-bit RGB rows, each with the Up filter, in one zlib stream
//! holding a single fixed-Huffman deflate block with LZ77 matches. Desktop frames repeat rows and
//! runs of one colour, which the Up filter and back-references compress without a Huffman tree.

const WINDOW: usize = 32 << 10;
const HASH_BITS: u32 = 15;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Encodes BGRX rows of `width` pixels as a PNG.
pub fn encode(width: u32, height: u32, bgrx: &[u8]) -> Vec<u8> {
    let stride = width as usize * 3;
    let mut rows = Vec::with_capacity((stride + 1) * height as usize);
    let mut previous = vec![0u8; stride];
    for row in bgrx.chunks_exact(width as usize * 4).take(height as usize) {
        let rgb: Vec<u8> = row
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect();
        rows.push(2); // Up
        rows.extend(
            rgb.iter()
                .zip(&previous)
                .map(|(&x, &up)| x.wrapping_sub(up)),
        );
        previous = rgb;
    }
    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib(&rows));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend((data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend(kind);
    png.extend(data);
    let crc = crc32(&png[start..]);
    png.extend(crc.to_be_bytes());
}

/// A zlib stream (RFC 1950) of one final fixed-Huffman block (RFC 1951 §3.2.6).
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut bits = Bits {
        out: vec![0x78, 0x01],
        buffer: 0,
        count: 0,
    };
    bits.put(0b011, 3); // BFINAL, BTYPE 01
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut at = 0;
    while at < data.len() {
        let (length, distance) = longest(data, at, &mut head);
        if length < MIN_MATCH {
            bits.literal(u32::from(data[at]));
            at += 1;
            continue;
        }
        bits.matched(length, distance);
        for skipped in at + 1..at + length {
            if skipped + MIN_MATCH <= data.len() {
                head[hash(data, skipped)] = skipped;
            }
        }
        at += length;
    }
    bits.literal(256);
    if bits.count > 0 {
        bits.out.push(bits.buffer as u8);
    }
    let mut out = bits.out;
    out.extend(adler32(data).to_be_bytes());
    out
}

/// The match at `at` against the last position with the same three bytes, recording `at`.
fn longest(data: &[u8], at: usize, head: &mut [usize]) -> (usize, usize) {
    if at + MIN_MATCH > data.len() {
        return (0, 0);
    }
    let slot = hash(data, at);
    let candidate = std::mem::replace(&mut head[slot], at);
    if candidate == usize::MAX || at - candidate > WINDOW {
        return (0, 0);
    }
    let limit = (data.len() - at).min(MAX_MATCH);
    let length = (0..limit)
        .take_while(|&offset| data[candidate + offset] == data[at + offset])
        .count();
    (length, at - candidate)
}

fn hash(data: &[u8], at: usize) -> usize {
    let word = u32::from(data[at]) | u32::from(data[at + 1]) << 8 | u32::from(data[at + 2]) << 16;
    (word.wrapping_mul(0x9e37_79b1) >> (32 - HASH_BITS)) as usize
}

/// Deflate's bit order: values least significant bit first, Huffman codes most significant first.
struct Bits {
    out: Vec<u8>,
    buffer: u64,
    count: u32,
}

impl Bits {
    fn put(&mut self, value: u32, length: u32) {
        self.buffer |= u64::from(value) << self.count;
        self.count += length;
        while self.count >= 8 {
            self.out.push(self.buffer as u8);
            self.buffer >>= 8;
            self.count -= 8;
        }
    }

    fn code(&mut self, code: u32, length: u32) {
        self.put(code.reverse_bits() >> (32 - length), length);
    }

    /// A literal byte, the end of block (256) or a length code (257 to 285).
    fn literal(&mut self, symbol: u32) {
        match symbol {
            0..=143 => self.code(0x30 + symbol, 8),
            144..=255 => self.code(0x190 + symbol - 144, 9),
            256..=279 => self.code(symbol - 256, 7),
            _ => self.code(0xc0 + symbol - 280, 8),
        }
    }

    fn matched(&mut self, length: usize, distance: usize) {
        let index = LENGTH_BASE.partition_point(|&base| usize::from(base) <= length) - 1;
        self.literal(257 + index as u32);
        let extra = u32::from(LENGTH_EXTRA[index]);
        self.put((length - usize::from(LENGTH_BASE[index])) as u32, extra);
        let index = DISTANCE_BASE.partition_point(|&base| usize::from(base) <= distance) - 1;
        self.code(index as u32, 5);
        let extra = u32::from(DISTANCE_EXTRA[index]);
        self.put((distance - usize::from(DISTANCE_BASE[index])) as u32, extra);
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), &byte| {
        let a = (a + u32::from(byte)) % 65521;
        (a, (b + a) % 65521)
    });
    b << 16 | a
}

/// CRC-32 (ISO-HDLC, as zlib and PNG compute it).
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |crc, &byte| {
        (0..8).fold(crc ^ u32::from(byte), |crc, _| {
            crc >> 1 ^ 0xedb8_8320 & (crc & 1).wrapping_neg()
        })
    })
}

/// Standard base64 with padding, as data URLs and provider image blocks take it.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let word = group.iter().enumerate().fold(0u32, |word, (index, &byte)| {
            word | u32::from(byte) << (16 - 8 * index)
        });
        for index in 0..4 {
            text.push(match index <= group.len() {
                true => ALPHABET[(word >> (18 - 6 * index) & 63) as usize] as char,
                false => '=',
            });
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Decodes with Python's zlib, an independent inflate, and returns the IDAT's raw rows.
    fn inflate(png: &[u8]) -> Vec<u8> {
        let length = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let mut python = std::process::Command::new("python3")
            .args(["-c", "import sys,zlib; sys.stdout.buffer.write(zlib.decompress(sys.stdin.buffer.read()))"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        python
            .stdin
            .take()
            .unwrap()
            .write_all(&png[41..41 + length])
            .unwrap();
        let output = python.wait_with_output().unwrap();
        assert!(output.status.success());
        output.stdout
    }

    #[test]
    fn encodes_rows_that_zlib_inflates_back() {
        let (width, height) = (300u32, 40u32);
        // Runs, a gradient and noise, so literals, short and long matches all occur.
        let mut seed = 7u32;
        let bgrx: Vec<u8> = (0..width * height)
            .flat_map(|pixel| {
                let (x, y) = (pixel % width, pixel / width);
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                match x / 100 {
                    0 => [10, 20, 30, 0],
                    1 => [x as u8, y as u8, (x + y) as u8, 0],
                    _ => [(seed >> 16) as u8, (seed >> 8) as u8, seed as u8, 0],
                }
            })
            .collect();
        let png = encode(width, height, &bgrx);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        let rows = inflate(&png);
        let stride = width as usize * 3;
        let mut previous = vec![0u8; stride];
        for (y, row) in rows.chunks(stride + 1).enumerate() {
            assert_eq!(row[0], 2);
            let rgb: Vec<u8> = row[1..]
                .iter()
                .zip(&previous)
                .map(|(&x, &up)| x.wrapping_add(up))
                .collect();
            let expected: Vec<u8> = bgrx[y * width as usize * 4..][..width as usize * 4]
                .chunks(4)
                .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
                .collect();
            assert_eq!(rgb, expected, "row {y}");
            previous = rgb;
        }
        // A blank screen is mostly back-references.
        let blank = encode(1280, 800, &vec![0; 1280 * 800 * 4]);
        assert!(blank.len() < 64 << 10, "{}", blank.len());
        assert_eq!(inflate(&blank).len(), 800 * (1280 * 3 + 1));
    }

    #[test]
    fn checksums_and_base64_match_their_standards() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
