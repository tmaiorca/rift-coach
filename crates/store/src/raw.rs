//! Compression for stored raw API responses. A timeline is ~1–2 MB of JSON
//! and shrinks roughly 10x.

const LEVEL: i32 = 7;

pub fn compress(json: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::encode_all(json, LEVEL)
}

pub fn decompress(body: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::decode_all(body)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips() {
        let data = br#"{"hello":"world","n":[1,2,3,1,2,3,1,2,3]}"#.repeat(100);
        let packed = super::compress(&data).expect("compress");
        assert!(packed.len() < data.len() / 5);
        assert_eq!(super::decompress(&packed).expect("decompress"), data);
    }
}
