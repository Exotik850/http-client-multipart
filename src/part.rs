use std::{borrow::Cow, io::Write, path::Path};

use async_fs::File as AsyncFile;
use base64::Engine;
use futures_lite::{io::BufReader, AsyncBufRead, AsyncReadExt, Stream, StreamExt};
use http_types::Body;
use mime_guess::Mime;

use crate::{reader_stream::ReaderStream, Encoding, StreamChunk};

/// Represents a single field in a multipart form.
#[derive(Debug)]
pub(crate) struct Part<'p> {
    name: Cow<'p, str>,
    data: Body,
    pub(crate) content_type: Mime,
    file_data: Option<Cow<'p, str>>,
    encoding: Option<Encoding>,
}

impl<'p> Part<'p> {
    /// Returns the filename of the part.
    /// Only applicable to file parts.
    pub(crate) fn filename(&self) -> Option<&str> {
        self.file_data.as_deref()
    }

    /// Returns the encoding of the part.
    /// Only applicable to file parts.
    pub(crate) fn encoding(&self) -> Option<Encoding> {
        self.encoding
    }

    /// Returns the data of the part as a stream.
    ///
    /// This is recommended for large files, as it will stream the file instead of loading it into memory.
    /// This also streams text values as bytes.
    ///
    /// Remember to place the boundary between parts when using this stream.
    pub(crate) fn into_stream(self, buf_size: Option<usize>) -> impl Stream<Item = StreamChunk> {
        let header = self.header_bytes();
        let header_stream = futures_lite::stream::once(Ok(header));
        let buf_size = buf_size.or(self.data.len());
        let encoding = self.encoding();
        let data = ReaderStream::new(self.data.into_reader(), buf_size, encoding);
        header_stream.chain(data)
    }

    pub(crate) fn into_reader(self, buf_size: Option<usize>) -> impl AsyncBufRead {
        let header = self.header_bytes();
        let header_reader = futures_lite::io::Cursor::new(header);
        let encoding = self.encoding();
        let buf_size = buf_size.or(self.data.len());
        let data_reader = self.data.into_reader();
        let data = ReaderStream::new(data_reader, buf_size, encoding);
        header_reader.chain(data)
    }

    /// Creates a new text part.
    pub(crate) fn text(
        name: impl Into<Cow<'p, str>>,
        value: impl AsRef<[u8]>,
        encoding: Option<Encoding>,
    ) -> Self {
        Part {
            name: name.into(),
            data: Body::from(value.as_ref()),
            content_type: "text/plain".parse().unwrap(),
            encoding,
            file_data: None,
        }
    }

    /// Creates a new file part.
    pub(crate) fn file_raw(
        name: impl Into<Cow<'p, str>>,
        filename: impl Into<Cow<'p, str>>,
        content_type: Mime,
        encoding: Option<Encoding>,
        data: impl Into<Body>,
    ) -> Self {
        Part {
            name: name.into(),
            data: data.into(),
            content_type,
            encoding,
            file_data: Some(filename.into()),
        }
    }

    /// Creates a new file part from a reader.
    pub(crate) fn file_raw_async(
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
            encoding,
            file_data: Some(filename.into()),
        }
    }

    /// Creates a new file part from a file.
    /// This will not load the entire file into memory,
    /// so it is recommended for large files.
    ///
    /// This tries to set the content type based on the file extension,
    /// falling back to `application/octet-stream` if the extension is not recognized.
    pub(crate) async fn file_async(
        name: impl Into<Cow<'p, str>>,
        path: impl AsRef<Path>,
        encoding: Option<Encoding>,
    ) -> Result<Self, futures_lite::io::Error> {
        let path = path.as_ref();
        let filename = filename(path);
        let content_type =
            content_type(path).unwrap_or_else(|| "application/octet-stream".parse().unwrap());
        let file = AsyncFile::open(path).await?;
        let mut data_len = file.metadata().await?.len() as usize;
        if let Some(Encoding::Base64) = encoding {
            // Base64 encoding increases the size of the data
            data_len = (data_len * 4 + 2) / 3; // Rough estimate for base64 size
        }
        let buf_reader = BufReader::new(file);
        Ok(Part::file_raw_async(
            name,
            filename,
            content_type,
            encoding,
            buf_reader,
            Some(data_len),
        ))
    }

    pub(crate) fn size_hint(&self) -> Option<usize> {
        let mut data_len = self.data.len()?;
        if let Some(Encoding::Base64) = self.encoding {
            data_len *= 4 / 3;
        }
        let header_len = self.header_len();
        Some((data_len + header_len))
    }

    fn header_len(&self) -> usize {
        // Calculate the length of the headers to be written
        // Initial part: "Content-Disposition: form-data; name=\"[name]\""
        let mut len = 39 + self.name.len(); // 41 = "Content-Disposition: form-data; name=\"\"".len()
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
        self.write_header(&mut header)
            .expect("Failed to write header");
        header
    }

    /// Extends the data of the part into a buffer.
    pub(crate) async fn extend(
        self,
        mut data: &mut Vec<u8>,
    ) -> Result<(), futures_lite::io::Error> {
        self.write_header(&mut data)?;
        let encoding = self.encoding;
        let mut stream = self.into_stream(None);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Some(Encoding::Base64) = encoding {
                // Encode if necessary (base64 encoding)
                let encoded_chunk = base64::engine::general_purpose::STANDARD.encode(&chunk);
                data.extend_from_slice(encoded_chunk.as_bytes());
                continue;
            }
            data.extend_from_slice(&chunk);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[async_std::test]
    async fn test_stream_and_reader_same_output() {
        let value = "This is a test value";
        let part_for_stream = Part::text("test_field", value, None);
        let part_for_reader = Part::text("test_field", value, None);

        // Collect bytes from the stream implementation.
        let mut stream = part_for_stream.into_stream(Some(8));
        let mut stream_output = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("stream chunk error");
            stream_output.extend(chunk);
        }

        // Collect bytes from the reader implementation.
        let mut reader = part_for_reader.into_reader(Some(8));
        let mut reader_output = Vec::new();
        reader
            .read_to_end(&mut reader_output)
            .await
            .expect("reader error");

        assert_eq!(stream_output, reader_output);
    }

    #[async_std::test]
    async fn test_part_size_hint_no_encoding() {
        let part = Part::text("field", "Hello world!", None);
        let expected_size = part.size_hint().unwrap();
        let mut buf = Vec::new();
        part.extend(&mut buf).await.expect("extend failed");
        assert_eq!(expected_size, buf.len());
    }

    #[async_std::test]
    async fn test_part_size_hint_base64_encoding() {
        let part = Part::text("field_base64", "Hello world!", Some(Encoding::Base64));
        let expected_size = part.size_hint().unwrap();
        let mut buf = Vec::new();
        part.extend(&mut buf).await.expect("extend failed");
        assert_eq!(expected_size, buf.len());
    }
}
