use crate::{
    generate_boundary,
    part::{ContentTransferEncoding, Part},
};
use async_fs::File as AsyncFile;
use base64::Engine;
use futures_lite::{io::BufReader, AsyncBufRead};
use http_types::{Body, Request, Result};
use std::{
    borrow::Cow,
    io::{Read, Seek},
    path::Path,
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
    pub fn add_text(&mut self, name: impl Into<Cow<'m, str>>, value: impl Into<Cow<'m, str>>) {
        self.fields.push(Part::text(name, value));
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
        content_type: impl Into<Cow<'m, str>>,
        encoding: Option<ContentTransferEncoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
    ) -> Result<()> {
        self.fields.push(Part::file_raw_async(
            name,
            filename,
            content_type,
            encoding,
            data,
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
        content_type: impl Into<Cow<'m, str>>,
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
        let body = self.to_body().await?;
        req.set_body(body);
        Ok(())
    }

    // pub fn into_stream(self) -> impl Stream<Item = Bytes> {
    //     futures_lite::future::
    // }

    /// Converts the multipart form to a `Body`.
    async fn to_body(self) -> Result<Body> {
        let mut data: Vec<u8> = Vec::new();

        for field in self.fields {
            // Add boundary for each field
            data.extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());

            match field {
                Part::Text { name, value } => {
                    // Add header and content for text fields
                    data.extend_from_slice(
                        format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name)
                            .as_bytes(),
                    );
                    data.extend_from_slice(value.as_bytes());
                }
                Part::File {
                    name,
                    filename,
                    content_type,
                    encoding,
                    data: file_data,
                } => {
                    // Add headers for file fields
                    generate_header_info(&mut data, name, filename, content_type, encoding);

                    // Process file content based on encoding
                    let mut content = file_data.into_bytes().await?;
                    if let Some(enc) = encoding {
                        content = enc.encode(content);
                    }
                    data.extend(content);
                }
            }
            data.extend_from_slice(b"\r\n");
        }

        // Add closing boundary
        data.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());

        Ok(Body::from(data))
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
