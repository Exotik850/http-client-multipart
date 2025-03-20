use std::{
    borrow::Cow,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use async_fs::File as AsyncFile;
use base64::Engine;
use futures_lite::{io::BufReader, AsyncBufRead, Stream, StreamExt};
use http_types::{Body, Mime};

use crate::StreamChunk;

/// Represents a single field in a multipart form.
#[derive(Debug)]
pub struct Part<'p> {
    name: Cow<'p, str>,
    data: Body,
    content_type: Mime,
    file_data: Option<FileData<'p>>,
}

#[derive(Debug)]
pub struct FileData<'p> {
    filename: Cow<'p, str>,
    encoding: Option<ContentTransferEncoding>,
}

impl<'p> Part<'p> {
    /// Returns the name of the part.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether the part is a text field.
    pub fn is_text(&self) -> bool {
        self.file_data.is_none()
    }

    /// Returns whether the part is a file field.
    pub fn is_file(&self) -> bool {
        self.file_data.is_some()
    }

    /// Returns the filename of the part.
    /// Only applicable to file parts.
    pub fn filename(&self) -> Option<&str> {
        self.file_data.as_ref().map(|data| data.filename.as_ref())
    }

    /// Returns the value of the part.
    /// This reads the entire part into memory,
    /// so it is not recommended for large files.
    pub async fn value(self) -> Result<Vec<u8>, http_types::Error> {
        self.data.into_bytes().await
    }

    /// Returns the content type of the part.
    /// Both text and file parts have a content type.
    /// Text parts have a default content type of `text/plain`.
    pub fn content_type(&self) -> &Mime {
        &self.content_type
    }

    /// Returns the encoding of the part.
    /// Only applicable to file parts.
    pub fn encoding(&self) -> Option<ContentTransferEncoding> {
        self.file_data.as_ref().and_then(|data| data.encoding)
    }

    /// Returns the data of the part as a stream.
    ///
    /// This is recommended for large files, as it will stream the file instead of loading it into memory.
    /// This also streams text values as bytes.
    pub fn into_stream(self) -> impl Stream<Item = StreamChunk> {
        let buf_size = self.data.len();
        let header = self.header().into_bytes();
        let data = ReaderStream::new(self.data.into_reader(), buf_size);
        futures_lite::stream::once(Ok(header)).chain(data)
    }

    /// Creates a new text part.
    pub fn text(name: impl Into<Cow<'p, str>>, value: &str) -> Self {
        Part {
            name: name.into(),
            data: Body::from(value),
            content_type: "text/plain".parse().unwrap(),
            file_data: None,
        }
    }

    /// Creates a new file part.
    pub fn file_raw(
        name: impl Into<Cow<'p, str>>,
        filename: impl Into<Cow<'p, str>>,
        content_type: Mime,
        encoding: Option<ContentTransferEncoding>,
        data: Body,
    ) -> Self {
        Part {
            name: name.into(),
            data,
            content_type,
            file_data: Some(FileData {
                filename: filename.into(),
                encoding,
            }),
        }
    }

    /// Creates a new file part from a reader.
    pub fn file_raw_async(
        name: impl Into<Cow<'p, str>>,
        filename: impl Into<Cow<'p, str>>,
        content_type: Mime,
        encoding: Option<ContentTransferEncoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
        buf_size: Option<usize>,
    ) -> Self {
        Part {
            name: name.into(),
            content_type,
            data: Body::from_reader(data, buf_size),
            file_data: Some(FileData {
                filename: filename.into(),
                encoding,
            }),
        }
    }

    /// Creates a new file part from a file.
    /// This will not load the entire file into memory,
    /// so it is recommended for large files.
    ///
    /// This tries to set the content type based on the file extension,
    /// falling back to `application/octet-stream` if the extension is not recognized.
    pub async fn file_async(
        name: impl Into<Cow<'p, str>>,
        path: impl AsRef<Path>,
        encoding: Option<ContentTransferEncoding>,
    ) -> Result<Self, futures_lite::io::Error> {
        let path = path.as_ref();
        let filename = filename(path);
        let content_type =
            content_type(path).unwrap_or_else(|| "application/octet-stream".parse().unwrap());
        let file = AsyncFile::open(path).await?;
        let buf_reader = BufReader::new(file);
        Ok(Part::file_raw_async(
            name,
            filename,
            content_type,
            encoding,
            buf_reader,
            None,
        ))
    }

    fn header(&self) -> String {
        let mut header = format!("Content-Disposition: form-data; name=\"{}\"", self.name);
        if let Some(filename) = self.filename() {
            header.push_str(&format!("; filename=\"{}\"", filename));
        }
        header.push_str("\r\n");
        header.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        if let Some(encoding) = self.encoding() {
            header.push_str(&format!(
                "Content-Transfer-Encoding: {}\r\n",
                encoding.to_str()
            ));
        }
        header.push_str("\r\n");
        header
    }

    /// Extends the data of the part into a buffer.
    pub async fn extend(self, data: &mut [u8]) -> Result<(), futures_lite::io::Error> {
        let header = self.header();
        let header_len = header.len();
        let header = header.into_bytes();
        data[..header_len].copy_from_slice(&header);
        let mut stream = self.into_stream();
        let mut offset = header_len;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let len = chunk.len();
            data[offset..offset + len].copy_from_slice(&chunk);
            offset += len;
        }
        Ok(())
    }
}

/// Returns the filename of a path.
/// If the path has no filename, it returns "file".
fn filename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into())
}

/// Returns a guessed content type based on a filename extension.
/// Returns `None` if the extension is not recognized.
fn content_type(path: &Path) -> Option<Mime> {
    let st = path.extension().and_then(|ext| ext.to_str())?;
    http_types::mime::Mime::from_extension(st)
}

struct ReaderStream<R> {
    reader: R,
    buf_size: Option<usize>,
}

impl<R: AsyncBufRead + Unpin + Send + Sync> ReaderStream<R> {
    fn new(reader: R, buf_size: Option<usize>) -> Self {
        Self { reader, buf_size }
    }
}

impl<R: AsyncBufRead + Unpin + Send + Sync> Stream for ReaderStream<R> {
    type Item = StreamChunk;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut buf = vec![0; self.buf_size.unwrap_or(1024)];
        let this = &mut self;
        let reader = Pin::new(&mut this.reader);
        match reader.poll_read(cx, &mut buf) {
            Poll::Ready(Ok(0)) => Poll::Ready(None),
            Poll::Ready(Ok(n)) => Poll::Ready(Some(Ok({
                buf.truncate(n);
                buf
            }))),
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    Base64,
    QuotedPrintable,
}

impl ContentTransferEncoding {
    pub fn to_str(self) -> &'static str {
        match self {
            ContentTransferEncoding::SevenBit => "7bit",
            ContentTransferEncoding::EightBit => "8bit",
            ContentTransferEncoding::Base64 => "base64",
            ContentTransferEncoding::QuotedPrintable => "quoted-printable",
        }
    }

    pub fn encode(self, input: Vec<u8>) -> Vec<u8> {
        match self {
            ContentTransferEncoding::Base64 => base64::engine::general_purpose::STANDARD_NO_PAD
                .encode(input)
                .into_bytes(),
            ContentTransferEncoding::QuotedPrintable => quoted_printable::encode(&input),
            ContentTransferEncoding::SevenBit | ContentTransferEncoding::EightBit => input,
        }
    }
}
