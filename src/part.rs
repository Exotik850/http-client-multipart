use std::{
    borrow::Cow,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use async_fs::File as AsyncFile;
use base64::Engine;
use bytes::Bytes;
use futures_lite::{io::BufReader, AsyncBufRead, Stream};
use http_types::Body;

/// Represents a single field in a multipart form.
#[derive(Debug)]
pub enum Part<'p> {
    /// A text field.
    Text {
        name: Cow<'p, str>,
        value: Cow<'p, str>,
    },
    /// A file field.
    File {
        name: Cow<'p, str>,
        filename: Cow<'p, str>,
        content_type: Cow<'p, str>,
        encoding: Option<ContentTransferEncoding>,
        data: Body,
    },
}

impl<'p> Part<'p> {
    /// Returns the name of the part.
    pub fn name(&self) -> &str {
        match self {
            Part::Text { name, .. } => name,
            Part::File { name, .. } => name,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Part::Text { .. })
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Part::File { .. })
    }

    /// Returns the filename of the part.
    /// Only applicable to file parts.
    pub fn filename(&self) -> Option<&str> {
        match self {
            Part::Text { .. } => None,
            Part::File { filename, .. } => Some(filename),
        }
    }

    /// Returns the value of the part.
    /// Only applicable to text parts.
    pub fn value(&self) -> Option<&str> {
        match self {
            Part::Text { value, .. } => Some(value),
            Part::File { .. } => None,
        }
    }

    /// Returns the content type of the part.
    /// Only applicable to file parts.
    pub fn content_type(&self) -> Option<&str> {
        match self {
            Part::Text { .. } => None,
            Part::File { content_type, .. } => Some(content_type),
        }
    }

    /// Returns the encoding of the part.
    /// Only applicable to file parts.
    pub fn encoding(&self) -> Option<ContentTransferEncoding> {
        match self {
            Part::Text { .. } => None,
            Part::File { encoding, .. } => *encoding,
        }
    }

    /// Returns the data of the part.
    ///
    /// This is not recommended for large files, as it will load the entire file into memory.
    /// 
    /// Returns `None` if the part is a text part.
    pub async fn into_data(
        self,
    ) -> Option<impl std::future::Future<Output = Result<Vec<u8>, http_types::Error>> + 'p> {
        if self.is_text() {
            return None;
        }
        let out = async {
            let out = match self {
                Part::File { data, .. } => data.into_bytes().await?,
                _ => unreachable!(),
            };
            Ok(out)
        };
        Some(out)
    }

    /// Returns the data of the part as a stream.
    /// 
    /// This is recommended for large files, as it will stream the file instead of loading it into memory.
    /// This also streams text values as bytes.
    pub fn into_stream(
        self,
    ) -> Box<dyn Stream<Item = std::result::Result<Bytes, futures_lite::io::Error>> + 'p> {
        todo!()
        // match self {
        //     Part::Text { value, .. } => {
        //         let value = value.into_owned();
        //         Box::new(futures_lite::stream::once(Ok(Bytes::from(value))))
        //     }
        //     Part::File { data, .. } => {
        //         let reader = ReaderStream::new(data.into_reader(), None);
        //         // Convert Stream<Item = Result<Bytes, Error>> to Stream<Item = Bytes>
        //         Box::new(reader)
        //     }
        // }
    }

    pub fn text(name: impl Into<Cow<'p, str>>, value: impl Into<Cow<'p, str>>) -> Self {
        Part::Text {
            name: name.into(),
            value: value.into(),
        }
    }

    pub fn file_raw(
        name: impl Into<Cow<'p, str>>,
        filename: impl Into<Cow<'p, str>>,
        content_type: impl Into<Cow<'p, str>>,
        encoding: Option<ContentTransferEncoding>,
        data: Body,
    ) -> Self {
        Part::File {
            name: name.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            encoding,
            data,
        }
    }

    pub fn file_raw_async(
        name: impl Into<Cow<'p, str>>,
        filename: impl Into<Cow<'p, str>>,
        content_type: impl Into<Cow<'p, str>>,
        encoding: Option<ContentTransferEncoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
    ) -> Self {
        Part::File {
            name: name.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            encoding,
            data: Body::from_reader(data, None),
        }
    }

    pub async fn file_async(
        name: impl Into<Cow<'p, str>>,
        path: impl AsRef<Path>,
        encoding: Option<ContentTransferEncoding>,
    ) -> Result<Self, futures_lite::io::Error> {
        let path = path.as_ref();
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());

        let content_type = mime_guess::from_path(path)
            .first()
            .map(|mime| mime.to_string())
            .unwrap_or_else(|| "application/octet-stream".into());

        let file = AsyncFile::open(path).await?;
        let buf_reader = BufReader::new(file);
        Ok(Part::file_raw_async(
            name,
            filename,
            content_type,
            encoding,
            buf_reader,
        ))
    }
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
    type Item = std::result::Result<Bytes, futures_lite::io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut buf = vec![0; self.buf_size.unwrap_or(1024)];
        let this = &mut self;
        let reader = Pin::new(&mut this.reader);
        match reader.poll_read(cx, &mut buf) {
            Poll::Ready(Ok(0)) => Poll::Ready(None),
            Poll::Ready(Ok(n)) => Poll::Ready(Some(Ok(Bytes::from({
                buf.truncate(n);
                buf
            })))),
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
