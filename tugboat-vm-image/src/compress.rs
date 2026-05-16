use crate::Error;
use std::io::{Read, Write};

pub(crate) fn compress_gzip(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data)?;
    Ok(e.finish()?)
}

pub(crate) fn decompress_gzip_to_writer<W: Write>(
    data: &[u8],
    writer: &mut W,
    max_uncompressed_bytes: u64,
) -> Result<u64, Error> {
    let mut d = flate2::read::GzDecoder::new(data);
    let mut buf = [0u8; 64 * 1024];
    let mut written = 0u64;

    loop {
        let len = d.read(&mut buf)?;
        if len == 0 {
            return Ok(written);
        }
        let len = u64::try_from(len).unwrap_or(u64::MAX);
        if written.saturating_add(len) > max_uncompressed_bytes {
            return Err(Error::ImageLayerTooLarge {
                limit: max_uncompressed_bytes,
            });
        }
        writer.write_all(&buf[..len as usize])?;
        written += len;
    }
}

#[cfg(test)]
mod tests {
    use super::{compress_gzip, decompress_gzip_to_writer};
    use crate::Error;

    #[test]
    fn decompresses_gzip_to_writer() {
        let data = compress_gzip(b"disk-data").expect("compress");
        let mut out = Vec::new();

        let written = decompress_gzip_to_writer(&data, &mut out, 1024).expect("decompress");

        assert_eq!(written, 9);
        assert_eq!(out, b"disk-data");
    }

    #[test]
    fn rejects_uncompressed_data_above_limit_without_writing_over_limit_chunk() {
        let data = compress_gzip(b"0123456789").expect("compress");
        let mut out = Vec::new();

        let err = decompress_gzip_to_writer(&data, &mut out, 5).expect_err("limit should fail");

        assert!(matches!(err, Error::ImageLayerTooLarge { limit: 5 }));
        assert!(out.len() <= 5);
    }
}
