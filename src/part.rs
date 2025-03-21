use std::{
    borrow::Cow,
    io::Write,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use async_fs::File as AsyncFile;
use base64::Engine;
use futures_lite::{io::BufReader, AsyncBufRead, AsyncRead, AsyncWrite, Stream, StreamExt};
use http_types::{Body};
use mime_guess::Mime;

use crate::StreamChunk;

const CHUNK_SIZE: usize = 1024;

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
    encoding: Option<Encoding>,
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
    pub fn encoding(&self) -> Option<Encoding> {
        self.file_data.as_ref().and_then(|data| data.encoding)
    }

    /// Returns the data of the part as a stream.
    ///
    /// This is recommended for large files, as it will stream the file instead of loading it into memory.
    /// This also streams text values as bytes.
    ///
    /// Remember to place the boundary between parts when using this stream.
    pub fn into_stream(self) -> impl Stream<Item = StreamChunk> {
        let header = self.header_bytes();
        let header_stream = futures_lite::stream::once(Ok(header));
        let buf_size = self.data.len();
        let encoding = self.encoding();
        let data = ReaderStream::new(self.data.into_reader(), buf_size, encoding);
        header_stream.chain(data)
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
        encoding: Option<Encoding>,
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
        encoding: Option<Encoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
        data_len: Option<usize>, // Optional length for the reader, if known
    ) -> Self {
        Part {
            name: name.into(),
            content_type,
            data: Body::from_reader(data, data_len),
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
        encoding: Option<Encoding>,
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

    fn header_len(&self) -> usize {
        // Calculate the length of the headers to be written
        // Initial part: "Content-Disposition: form-data; name=\"[name]\""
        let mut len = 41 + self.name.len(); // 41 = "Content-Disposition: form-data; name=\"\"".len()
        if let Some(filename) = self.filename() {
            // Add "; filename=\"[filename]\"" if this is a file part
            len += 15 + filename.len(); // 15 = "; filename=\"\"".len()
        }
        len += 2; // CRLF after Content-Disposition line
        // "Content-Type: [content_type]" line
        len += 14 + self.content_type.essence_str().len(); // 14 = "Content-Type: ".len()
        len += 2; // CRLF after Content-Type
        if let Some(encoding) = self.encoding() {
            // "Content-Transfer-Encoding: [encoding]" line
            len += 27 + encoding.to_str().len(); // 27 = "Content-Transfer-Encoding: ".len()
            len += 2; // CRLF after Content-Transfer-Encoding
        }
        len + 2 // Final CRLF that separates headers from body
    }

    fn write_header<W: std::io::Write>(&self, mut buf: W) -> Result<(), std::io::Error> {
        buf.write_all(
            format!("Content-Disposition: form-data; name=\"{}\"", self.name).as_bytes(),
        )?;
        if let Some(filename) = self.filename() {
            buf.write_all(format!("; filename=\"{}\"", filename).as_bytes())?;
        }
        buf.write_all(b"\r\n")?;
        buf.write_all(format!("Content-Type: {}\r\n", self.content_type).as_bytes())?;
        if let Some(encoding) = self.encoding() {
            buf.write_all(
                format!("Content-Transfer-Encoding: {}\r\n", encoding.to_str()).as_bytes(),
            )?;
        }
        buf.write_all(b"\r\n")?; // Blank line to separate headers from body
        Ok(())
    }

    fn header_bytes(&self) -> Vec<u8> {
        let mut header = Vec::with_capacity(self.header_len());
        self.write_header(&mut header).expect("Failed to write header");
        header
    }

    /// Extends the data of the part into a buffer.
    pub async fn extend(self, mut data: &mut [u8]) -> Result<(), futures_lite::io::Error> {
        self.write_header(&mut data)?;
        let mut stream = self.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            data.write_all(&chunk)?;
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
    mime_guess::from_path(path).first()
}

struct ReaderStream<R> {
    reader: R,
    buf_size: usize,
    encoding: Option<Encoding>,
}

impl<R: AsyncRead + Unpin + Send + Sync> ReaderStream<R> {
    fn new(reader: R, buf_size: Option<usize>, encoding: Option<Encoding>) -> Self {
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

#[derive(Debug, Clone, Copy)]
pub enum Encoding {
    SevenBit,
    EightBit,
    Base64,
    QuotedPrintable,
}

impl Encoding {
    pub fn to_str(self) -> &'static str {
        match self {
            Encoding::SevenBit => "7bit",
            Encoding::EightBit => "8bit",
            Encoding::Base64 => "base64",
            Encoding::QuotedPrintable => "quoted-printable",
        }
    }

    pub fn encode(self, input: Vec<u8>) -> Vec<u8> {
        match self {
            Encoding::Base64 => base64::engine::general_purpose::STANDARD_NO_PAD
                .encode(input)
                .into_bytes(),
            Encoding::QuotedPrintable => quoted_printable::encode(&input),
            Encoding::SevenBit | Encoding::EightBit => input,
        }
    }
}
