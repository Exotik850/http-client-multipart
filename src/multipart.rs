use crate::{generate_boundary, part::Part, Encoding, StreamChunk};
use futures_lite::{AsyncBufRead, AsyncReadExt, Stream, StreamExt};
use http_types::{Body, Request, Result};
use std::{
    borrow::Cow,
    io::{Read, Seek},
    path::Path,
    pin::Pin,
};

/// A struct representing a multipart form.
#[derive(Debug)]
pub struct Multipart<'m> {
    boundary: String,
    fields: Vec<Part<'m>>,
}

impl Default for Multipart<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'m> Multipart<'m> {
    /// Creates a new `Multipart` form with a randomly generated boundary.
    pub fn new() -> Self {
        Self {
            boundary: generate_boundary(),
            fields: Vec::new(),
        }
    }

    /// Adds a text field to the form.
    pub fn add_text(&mut self, name: impl Into<Cow<'m, str>>, value: impl AsRef<str>) {
        self.fields.push(Part::text(name, value.as_ref(), None));
    }

    pub fn add_enc_text(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        value: impl AsRef<str>,
        encoding: Encoding,
    ) {
        self.fields
            .push(Part::text(name, value.as_ref(), Some(encoding)));
    }

    /// Adds a text field to the form with a custom mime type.
    ///
    /// Returns an error if the mime type is invalid.
    pub fn add_text_mime(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        value: impl AsRef<str>,
        mime: &str,
    ) -> Result<()> {
        let mut part = Part::text(name, value.as_ref(), None);
        part.content_type = mime.parse()?;
        self.fields.push(part);
        Ok(())
    }

    pub fn add_file_bytes(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        filename: impl Into<Cow<'m, str>>,
        value: impl AsRef<[u8]>,
        mime: &str,
        encoding: Option<Encoding>,
    ) -> Result<()> {
        let content_type = mime.parse()?;
        let part = Part::file_raw(name, filename, content_type, encoding, value.as_ref());
        self.fields.push(part);
        Ok(())
    }

    /// Adds a file field to the form from path.
    pub async fn add_file(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        path: impl AsRef<Path>,
        encoding: Option<Encoding>,
    ) -> Result<()> {
        let part = Part::file_async(name, path, encoding).await?;
        self.fields.push(part);
        Ok(())
    }

    /// Adds a file field to the form wrapping a async reader.
    pub fn add_async_read(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        filename: impl Into<Cow<'m, str>>,
        content_type: &str,
        encoding: Option<Encoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
        data_len: Option<usize>, // optional length for the async reader, if known
    ) -> Result<()> {
        self.fields.push(Part::file_raw_async(
            name,
            filename,
            content_type.parse()?,
            encoding,
            data,
            data_len,
        ));
        Ok(())
    }

    /// Adds a file field to the form wrapping a sync reader.
    pub fn add_sync_read(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        filename: impl Into<Cow<'m, str>>,
        content_type: &str,
        encoding: Option<Encoding>,
        mut data: impl Read + Seek + Send + 'static,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        data.read_to_end(&mut buffer)?;
        let body = Body::from(buffer);
        self.fields.push(Part::file_raw(
            name,
            filename,
            content_type.parse()?,
            encoding,
            body,
        ));
        Ok(())
    }

    /// Sets the request body to the multipart form data.
    pub fn set_request(self, req: &mut Request) {
        let content_type = format!("multipart/form-data; boundary={}", &self.boundary);
        req.insert_header("Content-Type", content_type);

        if let Some(size) = self.size_hint() {
            req.insert_header("Content-Length", size.to_string());
        } else {
            // If size not known, unset Content-Length header
            req.remove_header("Content-Length");
        }

        let body = self.into_body(None);
        req.set_body(body);
    }

    pub async fn set_request_bytes(self, req: &mut Request) -> Result<()> {
        let content_type = format!("multipart/form-data; boundary={}", &self.boundary);
        req.insert_header("Content-Type", content_type);

        if let Some(size) = self.size_hint() {
            req.insert_header("Content-Length", size.to_string());
        } else {
            // If size not known, unset Content-Length header
            req.remove_header("Content-Length");
        }

        let body = self.into_body_bytes().await?;
        req.set_body(body);
        Ok(())
    }

    /// Converts the multipart form to a `Body`.
    pub async fn into_bytes(self) -> Result<Vec<u8>> {
        let mut data: Vec<u8> = Vec::new();

        for field in self.fields {
            // Add boundary for each field
            data.extend(format!("--{}\r\n", self.boundary).into_bytes());
            field.extend(&mut data).await?;
        }
        data.extend_from_slice(b"\r\n");

        // Add closing boundary
        data.extend(format!("--{}--\r\n", self.boundary).into_bytes());

        Ok(data)
    }

    pub fn into_stream(self, buf_size: Option<usize>) -> impl Stream<Item = StreamChunk> {
        if self.fields.is_empty() {
            let empty_stream: Pin<Box<dyn Stream<Item = StreamChunk>>> =
                Box::pin(futures_lite::stream::empty());
            return empty_stream;
        }

        let head_bytes = format!("--{}\r\n", self.boundary).into_bytes();
        let head_stream = futures_lite::stream::once(Ok(head_bytes.clone()));
        let seperator = format!("\r\n--{}\r\n", self.boundary).into_bytes();
        let mut field_iter = self.fields.into_iter();
        let start = field_iter.next().unwrap().into_stream(buf_size);
        let start = Box::pin(head_stream.chain(start)) as Pin<Box<dyn Stream<Item = StreamChunk>>>;
        let stream = field_iter.fold(start, |acc, field| {
            let seperator = futures_lite::stream::once(Ok(seperator.clone()));
            let stream = field.into_stream(buf_size);
            Box::pin(acc.chain(seperator).chain(stream)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
        });
        let tail = format!("\r\n--{}--\r\n", self.boundary).into_bytes();
        let end = futures_lite::stream::once(Ok(tail));
        Box::pin(stream.chain(end)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
    }

    pub fn into_reader(self, buf_size: Option<usize>) -> impl AsyncBufRead + Send + Sync {
        if self.fields.is_empty() {
            return Box::pin(futures_lite::io::empty()) as Pin<Box<dyn AsyncBufRead + Send + Sync>>;
        }

        let head_bytes = format!("--{}\r\n", self.boundary).into_bytes();
        let header_reader = futures_lite::io::Cursor::new(head_bytes.clone());
        let seperator = format!("\r\n--{}\r\n", self.boundary).into_bytes();

        let mut field_iter = self.fields.into_iter();
        let start = field_iter.next().unwrap().into_reader(buf_size);
        let start =
            Box::pin(header_reader.chain(start)) as Pin<Box<dyn AsyncBufRead + Send + Sync>>;
        let reader = field_iter.fold(start, |acc, field| {
            let seperator = futures_lite::io::Cursor::new(seperator.clone());
            let reader = field.into_reader(buf_size);
            Box::pin(acc.chain(seperator).chain(reader)) as Pin<Box<dyn AsyncBufRead + Send + Sync>>
        });
        let tail = format!("\r\n--{}--\r\n", self.boundary).into_bytes();
        let end = futures_lite::io::Cursor::new(tail);
        Box::pin(reader.chain(end)) as Pin<Box<dyn AsyncBufRead + Send + Sync>>
    }

    fn size_hint(&self) -> Option<usize> {
        // The first seperator is 30 + 2 + 2 = 34 bytes
        // The last seperator is 30 + 2 + 2 + 2 + 2 = 38 bytes
        // The seperator between fields is 30 + 2 + 2 + 2 = 36 bytes
        // The total size is 34 + 36 * (n - 1) + 38 = 36 * n + 2

        let mut size = 34;
        for field in &self.fields {
            size += 36;
            size += field.size_hint()?;
        }
        Some(size + 38)
    }

    fn into_body(self, buf_size: Option<usize>) -> Body {
        let hint = self.size_hint();
        Body::from_reader(self.into_reader(buf_size), hint)
    }

    async fn into_body_bytes(self) -> Result<Body> {
        let bytes = self.into_bytes().await?;
        Ok(Body::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a Multipart with fixed boundary and sample text fields.
    fn create_multipart<'a>() -> Multipart<'a> {
        let mut m = Multipart::new();
        // Override the randomly generated boundary for consistency in tests.
        m.boundary = "test-boundary".into();
        m.add_text("field1", "value1");
        m.add_text("field2", "value2");
        m
    }

    #[async_std::test]
    async fn test_stream_and_reader_equivalence() -> Result<()> {
        // Create two identical Multipart instances.
        let m_stream = create_multipart();
        let m_reader = create_multipart();

        // Collect output from the stream implementation.
        let mut stream = m_stream.into_stream(Some(8));
        let mut stream_output = Vec::new();
        while let Some(chunk) = stream.next().await {
            stream_output.extend(chunk?);
        }

        // Collect output from the reader implementation.
        let mut reader = m_reader.into_reader(Some(8));
        let mut reader_output = Vec::new();
        reader.read_to_end(&mut reader_output).await?;

        // Compare both outputs.
        assert_eq!(stream_output, reader_output);

        Ok(())
    }

    #[async_std::test]
    async fn test_file_and_text_equivalence() -> Result<()> {
        // The content to write to the file.
        let content = "Hello, world!";

        // Create a temporary file and write the content into it.
        let file_path = "./examples/file.txt";

        // Create a Multipart instance by loading the file using base64 encoding.
        let mut m_file = Multipart::new();
        // Override the boundary for consistency in tests.
        m_file.boundary = "test-boundary".into();
        m_file
            .add_file("filefield", file_path, Some(Encoding::Base64))
            .await
            .unwrap();

        // Create a Multipart instance by preloading the file content as text,
        // setting a custom mime ('text/plain') and using base64 encoding.
        let mut m_text = Multipart::new();
        m_text.boundary = "test-boundary".into();
        m_text
            .add_file_bytes(
                "filefield",
                "file.txt",
                content,
                "text/plain",
                Some(Encoding::Base64),
            )
            .unwrap();

        // Convert both multipart forms to bytes.
        let bytes_file = m_file.into_bytes().await.unwrap();
        let bytes_text = m_text.into_bytes().await.unwrap();

        // Assert that both outputs are identical.
        assert_eq!(
            String::from_utf8_lossy(&bytes_file),
            String::from_utf8_lossy(&bytes_text)
        );

        Ok(())
    }

    #[async_std::test]
    async fn test_multipart_size_hint() {
        // Hypothetical usage, depending on your actual Multipart API.
        let mut multipart = Multipart::new();
        multipart.boundary = "test-boundary".into();
        multipart.add_text("field", "Hello multipart!");
        let expected_size = multipart.size_hint().unwrap();

        let buf = multipart.into_bytes().await.unwrap();
        assert_eq!(expected_size, buf.len());
    }
}
