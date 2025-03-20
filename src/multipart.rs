use crate::{
    generate_boundary,
    part::{ContentTransferEncoding, Part},
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
        encoding: Option<ContentTransferEncoding>,
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
        content_type: Mime,
        encoding: Option<ContentTransferEncoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
        buf_len: Option<usize>,
    ) -> Result<()> {
        self.fields.push(Part::file_raw_async(
            name,
            filename,
            content_type,
            encoding,
            data,
            buf_len,
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
        content_type: Mime,
        encoding: Option<ContentTransferEncoding>,
        mut data: impl Read + Seek + Send + 'static,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        data.read_to_end(&mut buffer)?;
        let body = Body::from(buffer);
        self.fields
            .push(Part::file_raw(name, filename, content_type, encoding, body));
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
            let out: Pin<Box<dyn Stream<Item = StreamChunk>>> =
                Box::pin(futures_lite::stream::empty());
            return out;
        }
        let head = format!("--{}\r\n", self.boundary).into_bytes();
        let head = futures_lite::stream::once(Ok(head));
        let mut field_iter = self.fields.into_iter();
        let start = field_iter.next().unwrap().into_stream();
        let start = Box::pin(head.chain(start)) as Pin<Box<dyn Stream<Item = StreamChunk>>>;
        let stream = field_iter.fold(start, |acc, field| {
            let stream = field.into_stream();
            Box::pin(acc.chain(stream)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
        });
        let closer = format!("--{}--\r\n", self.boundary).into_bytes();
        let end = futures_lite::stream::once(Ok(closer));
        Box::pin(stream.chain(end)) as Pin<Box<dyn Stream<Item = StreamChunk>>>
    }
}

fn generate_header_info(
    data: &mut Vec<u8>,
    name: Cow<'_, str>,
    filename: Cow<'_, str>,
    content_type: Cow<'_, str>,
    encoding: Option<ContentTransferEncoding>,
) {
    let headers = [
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
            name, filename
        ),
        format!("Content-Type: {}\r\n", content_type),
        encoding.map_or(String::new(), |enc| {
            format!("Content-Transfer-Encoding: {}\r\n", enc.to_str())
        }),
    ];

    for header in headers {
        if !header.is_empty() {
            data.extend_from_slice(header.as_bytes());
        }
    }

    data.extend_from_slice(b"\r\n");
}
