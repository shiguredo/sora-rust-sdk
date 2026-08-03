//! zlib 圧縮と展開の補助。
use std::io;

pub(crate) fn compress_zlib(data: &[u8]) -> io::Result<Vec<u8>> {
    noflate::zlib::compress(data).map_err(io::Error::other)
}

pub(crate) fn decompress_zlib(data: &[u8], max_size: usize) -> io::Result<Vec<u8>> {
    // 圧縮入力を 4 KiB の固定チャンクに分割して feed し、
    // 各回の出力長を累積検査することで展開後サイズを上限以下に制限する。
    // noflate の Decoder は 1 回の feed 中に出力上限で処理を中断する API を持たないため、
    // 一時メモリは 16 MiB・4 KiB の圧縮入力 1 チャンクから生成される有界な出力・
    // decoder の作業領域の合計に抑える。
    const CHUNK_SIZE: usize = 4 * 1024;

    let mut decoder = noflate::zlib::Decoder::new();
    let mut output = Vec::new();
    let mut total_output = 0usize;

    for chunk in data.chunks(CHUNK_SIZE) {
        decoder.feed(chunk).map_err(io::Error::other)?;
        let produced = decoder.output();
        let new_total = total_output.checked_add(produced.len()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "decompressed size overflow")
        })?;
        if new_total > max_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decompressed size exceeds limit",
            ));
        }
        output.extend_from_slice(produced);
        total_output = new_total;
        decoder.advance(produced.len());
    }

    // 全入力の供給後に trailer まで完了していない入力はエラーにする
    if !decoder.is_finished() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zlib stream ended before the trailer",
        ));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_zlib_within_limit() {
        let original = b"Hello, zlib!";
        let compressed = compress_zlib(original).unwrap();
        let decompressed = decompress_zlib(&compressed, 1024).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_zlib_at_limit() {
        let original = vec![b'a'; 1024];
        let compressed = compress_zlib(&original).unwrap();
        let decompressed = decompress_zlib(&compressed, 1024).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_zlib_over_limit() {
        let original = vec![b'a'; 1025];
        let compressed = compress_zlib(&original).unwrap();
        let err = decompress_zlib(&compressed, 1024).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decompress_zlib_high_compression_ratio() {
        // 高圧縮率の反復データは小さい圧縮入力から大量の出力を生む
        let original = vec![b'a'; 1024 * 1024];
        let compressed = compress_zlib(&original).unwrap();
        assert!(compressed.len() < 1024 * 1024);
        let err = decompress_zlib(&compressed, 1024).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decompress_zlib_empty_payload_with_limit_zero() {
        // 空ペイロードを表す正常な zlib ストリームは出力 0 バイトのため、
        // 上限 0 でも展開できる
        let compressed = compress_zlib(b"").unwrap();
        let decompressed = decompress_zlib(&compressed, 0).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn decompress_zlib_truncated_stream() {
        let original = b"Hello, zlib!";
        let compressed = compress_zlib(original).unwrap();
        let truncated = &compressed[..compressed.len() - 1];
        let err = decompress_zlib(truncated, 1024).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decompress_zlib_adler32_mismatch() {
        let original = b"Hello, zlib!";
        let mut compressed = compress_zlib(original).unwrap();
        let last = compressed.len() - 1;
        compressed[last] ^= 0xFF;
        let err = decompress_zlib(&compressed, 1024).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }
}
