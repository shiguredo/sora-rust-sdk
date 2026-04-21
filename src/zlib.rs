//! zlib 圧縮と展開の補助。
use std::io;

pub(crate) fn compress_zlib(data: &[u8]) -> io::Result<Vec<u8>> {
    noflate::zlib::compress(data).map_err(io::Error::other)
}

pub(crate) fn decompress_zlib(data: &[u8]) -> io::Result<Vec<u8>> {
    noflate::zlib::decompress(data).map_err(io::Error::other)
}
