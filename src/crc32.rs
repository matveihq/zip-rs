//! Helper module to compute a CRC32 checksum

use std::io;
use std::io::prelude::*;

use crc32fast::Hasher;

#[cfg(feature = "async")]
use futures::io::AsyncRead;
#[cfg(feature = "async")]
use pin_project::pin_project;

/// Reader that validates the CRC32 when it reaches the EOF.
#[cfg_attr(feature = "async", pin_project(project=Crc32ReaderProject))]
pub struct Crc32Reader<R> {
    #[cfg_attr(feature = "async", pin)]
    inner: R,
    hasher: Hasher,
    check: u32,
}

impl<R> Crc32Reader<R> {
    /// Get a new Crc32Reader which check the inner reader against checksum.
    pub fn new(inner: R, checksum: u32) -> Crc32Reader<R> {
        Crc32Reader {
            inner,
            hasher: Hasher::new(),
            check: checksum,
        }
    }

    fn check_matches(&self) -> bool {
        self.check == self.hasher.clone().finalize()
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for Crc32Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = match self.inner.read(buf) {
            Ok(0) if !buf.is_empty() && !self.check_matches() => {
                return Err(io::Error::new(io::ErrorKind::Other, "Invalid checksum"))
            }
            Ok(n) => n,
            Err(e) => return Err(e),
        };
        self.hasher.update(&buf[0..count]);
        Ok(count)
    }
}

#[cfg(feature = "async")]
impl<R: AsyncRead> AsyncRead for Crc32Reader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let Crc32ReaderProject {
            inner,
            hasher,
            check,
        } = self.as_mut().project();
        inner.poll_read(cx, buf).map(|count| {
            let count = match count {
                Ok(0) if !buf.is_empty() && !(*check == hasher.clone().finalize()) => {
                    return Err(io::Error::new(io::ErrorKind::Other, "Invalid checksum"))
                }
                Ok(n) => n,
                Err(e) => return Err(e),
            };
            hasher.update(&buf[0..count]);
            Ok(count)
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_empty_reader() {
        let data: &[u8] = b"";
        let mut buf = [0; 1];

        let mut reader = Crc32Reader::new(data, 0);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);

        let mut reader = Crc32Reader::new(data, 1);
        assert!(reader
            .read(&mut buf)
            .unwrap_err()
            .to_string()
            .contains("Invalid checksum"));
    }

    #[test]
    fn test_byte_by_byte() {
        let data: &[u8] = b"1234";
        let mut buf = [0; 1];

        let mut reader = Crc32Reader::new(data, 0x9be3e0a3);
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
        assert_eq!(reader.read(&mut buf).unwrap(), 1);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
        // Can keep reading 0 bytes after the end
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn test_zero_read() {
        let data: &[u8] = b"1234";
        let mut buf = [0; 5];

        let mut reader = Crc32Reader::new(data, 0x9be3e0a3);
        assert_eq!(reader.read(&mut buf[..0]).unwrap(), 0);
        assert_eq!(reader.read(&mut buf).unwrap(), 4);
    }
}
