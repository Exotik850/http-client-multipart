use crate::{
    generate_boundary,
    part::{Encoding, Part},
    StreamChunk,
};
use futures_lite::{AsyncBufRead, Stream, StreamExt};
use http_types::{Body, Mime, Request, Result};
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
        self.fields.push(Part::text(name, value.as_ref()));
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

    pub fn add_part(&mut self, part: Part<'m>) {
        self.fields.push(part);
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
    pub async fn set_request(self, req: &mut Request) -> Result<()> {
        let content_type = format!("multipart/form-data; boundary={}", &self.boundary);
        req.insert_header("Content-Type", content_type);
        let body = Body::from_bytes(self.into_bytes().await?);
        req.set_body(body);
        Ok(())
    }

    /// Converts the multipart form to a `Body`.
    async fn into_bytes(self) -> Result<Vec<u8>> {
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

    pub fn into_stream(self) -> impl Stream<Item = StreamChunk> {
        if self.fields.is_empty() {
            let empty_stream: Pin<Box<dyn Stream<Item = StreamChunk>>> =
                Box::pin(futures_lite::stream::empty());
            return empty_stream;
        }

        let head_bytes = format!("--{}\r\n", self.boundary).into_bytes();
        let head_stream = futures_lite::stream::once(Ok(head_bytes.clone()));
        let mut field_iter = self.fields.into_iter();
        let start = field_iter.next().unwrap().into_stream();
        let start = Box::pin(head_stream.chain(start)) as Pin<Box<dyn Stream<Item = StreamChunk>>>;
        let stream = field_iter.fold(start, |acc, field| {
            let seperator = futures_lite::stream::once(Ok(head_bytes.clone()));
            let stream = field.into_stream();
            Box::pin(acc.chain(seperator).chain(stream)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
        });
        let tail = format!("--{}--\r\n", self.boundary).into_bytes();
        let end = futures_lite::stream::once(Ok(tail));
        Box::pin(stream.chain(end)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
    }
}
