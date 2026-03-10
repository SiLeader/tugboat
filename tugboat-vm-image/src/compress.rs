use crate::Error;

pub(crate) fn compress_gzip(data: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Write;

    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data)?;
    Ok(e.finish()?)
}

pub(crate) fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Read;

    let mut d = flate2::read::GzDecoder::new(data);
    let mut result = Vec::new();
    d.read_to_end(&mut result)?;
    Ok(result)
}
