use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_lite::{AsyncBufRead, AsyncRead, Stream};

use crate::{Encoding, StreamChunk};

const CHUNK_SIZE: usize = 2048;

pub(crate) struct ReaderStream<R> {
    reader: R,
    buf_size: usize,
    encoding: Option<Encoding>,
}

impl<R: AsyncRead + Unpin + Send + Sync> ReaderStream<R> {
    pub(crate) fn new(reader: R, buf_size: Option<usize>, encoding: Option<Encoding>) -> Self {
        Self {
            reader,
            buf_size: buf_size.unwrap_or(CHUNK_SIZE),
            encoding,
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
                buf.truncate(dbg!(n)); // Resize to actual bytes read
                if let Some(encoding) = this.encoding {
                    let encoded = encoding.encode(buf);
                    Poll::Ready(Some(Ok(encoded)))
                } else {
                    Poll::Ready(Some(Ok(buf)))
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}
