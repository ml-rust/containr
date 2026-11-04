//! A target with no seek, such as a pipe or socket.

use std::io::{self, Seek, SeekFrom, Write};

/// A [`Write`] target with a [`Seek`] that always fails.
///
/// Use it with `MkvOptions::seekable` set to false: the muxer then never
/// seeks. With `seekable` set to true, `Muxer::finish` returns the seek error.
#[derive(Debug)]
pub struct NoSeek<W>(pub W);

impl<W: Write> Write for NoSeek<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<W> Seek for NoSeek<W> {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this target has no seek. Set `MkvOptions::seekable` to false",
        ))
    }
}
