use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_lite::{AsyncBufRead, AsyncRead, Stream};

use crate::{Encoding, StreamChunk};

const CHUNK_SIZE: usize = 256;

pub(crate) struct ReaderStream<R> {
    reader: R,
    buf_size: usize,
    buf_buffer: Option<Vec<u8>>,
    encoding: Option<Encoding>,
}

pub fn nearest_multiple_of(n: usize, multiple: usize) -> usize {
    if n % multiple == 0 {
        n
    } else {
        (n / multiple + 1) * multiple
    }
}

impl<R: AsyncBufRead + Unpin + Send + Sync> ReaderStream<R> {
    pub(crate) fn new(reader: R, buf_size: Option<usize>, encoding: Option<Encoding>) -> Self {
        let mut buf_size = buf_size.unwrap_or(CHUNK_SIZE);
        if let Some(Encoding::Base64) = encoding {
            // Base64 encoding requires a buffer size that is a multiple of 3
            buf_size = nearest_multiple_of(buf_size, 3);
        }
        Self {
            reader,
            buf_size,
            encoding,
            buf_buffer: None,
        }
    }
}

impl<R: AsyncBufRead + Unpin + Send + Sync> Stream for ReaderStream<R> {
    type Item = StreamChunk;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let buf_size = self.buf_size;
        let mut buf = vec![0; buf_size];
        let this = &mut self;
        let reader = Pin::new(&mut this.reader);

        match reader.poll_read(cx, &mut buf) {
            Poll::Ready(Ok(0)) => Poll::Ready(None), // EOF
            Poll::Ready(Ok(n)) => {
                buf.truncate(n); // Resize to actual bytes read
                if let Some(encoding) = this.encoding {
                    encoding.encode(&mut buf);
                    Poll::Ready(Some(Ok(buf)))
                } else {
                    Poll::Ready(Some(Ok(buf)))
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R: AsyncBufRead + Unpin + Send + Sync> AsyncRead for ReaderStream<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.encoding.is_some() {
            // When encoding is needed, we need to use the stream implementation
            // and cannot directly pass through to the reader
            let mut temp_buf = vec![0; buf.len()];
            let reader = Pin::new(&mut this.reader);
            match reader.poll_read(cx, &mut temp_buf) {
                Poll::Ready(Ok(0)) => Poll::Ready(Ok(0)), // EOF
                Poll::Ready(Ok(n)) => {
                    temp_buf.truncate(n); // Resize to actual bytes read
                    if let Some(encoding) = &this.encoding {
                        encoding.encode(&mut temp_buf);
                        let copy_size = std::cmp::min(temp_buf.len(), buf.len());
                        buf[..copy_size].copy_from_slice(&temp_buf[..copy_size]);
                        Poll::Ready(Ok(copy_size))
                    } else {
                        let copy_size = std::cmp::min(n, buf.len());
                        buf[..copy_size].copy_from_slice(&temp_buf[..copy_size]);
                        Poll::Ready(Ok(copy_size))
                    }
                }
                other => other,
            }
        } else {
            // When no encoding is needed, pass through directly
            let reader = Pin::new(&mut this.reader);
            reader.poll_read(cx, buf)
        }
    }
}

impl<R: AsyncBufRead + Unpin + Send + Sync> AsyncBufRead for ReaderStream<R> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<&[u8]>> {
        // let this = self.get_mut();
        // let reader = Pin::new(&mut this.reader);
        // reader.poll_fill_buf(cx)

        let this = self.get_mut();
        if this.encoding.is_none() {
            let reader = Pin::new(&mut this.reader);
            return reader.poll_fill_buf(cx);
        }
        let buf_size = this.buf_size;
        let buf = this.buf_buffer.get_or_insert_with(|| vec![0; buf_size]);
        let reader = Pin::new(&mut this.reader);
        match reader.poll_read(cx, buf) {
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(&[])), // EOF
            Poll::Ready(Ok(n)) => {
                buf.truncate(n);
                let encoding = this.encoding.unwrap();
                encoding.encode(buf);
                Poll::Ready(Ok(this.buf_buffer.as_ref().unwrap()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.get_mut();
        let reader = Pin::new(&mut this.reader);
        reader.consume(amt)
    }
}
