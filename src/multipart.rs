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
        self.fields.push(Part::Text {
            name: name.into(),
            value: value.into(),
        });
    }

    /// Adds a file field to the form from path.
    pub async fn add_file(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        path: impl AsRef<Path>,
        encoding: Option<ContentTransferEncoding>,
    ) -> Result<()> {
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
        self.add_async_read(name, filename, content_type, encoding, buf_reader)
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
        let body = Body::from_reader(data, None);
        self.fields.push(Part::File {
            name: name.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            encoding,
            data: body,
        });
        Ok(())
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
        let body = Body::from_bytes(buffer);
        self.fields.push(Part::File {
            name: name.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            encoding,
            data: body,
        });
        Ok(())
    }

    /// Adds a file field to the form from a `File` object.
    pub fn add_file_from_sync(
        &mut self,
        name: impl Into<Cow<'m, str>>,
        filename: impl Into<Cow<'m, str>>,
        content_type: impl Into<Cow<'m, str>>,
        encoding: Option<ContentTransferEncoding>,
        file: std::fs::File,
    ) -> Result<()> {
        self.add_sync_read(name, filename, content_type, encoding, file)
    }

    /// Sets the request body to the multipart form data.
    pub async fn set_request(self, req: &mut Request) -> Result<()> {
        let boundary = self.boundary.clone();
        let body = self.to_body().await?;
        req.set_body(body);
        let content_type = format!("multipart/form-data; boundary={}", boundary);
        req.insert_header("Content-Type", content_type);
        Ok(())
    }

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
              format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
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
            let headers = [
              format!("Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n", name, filename),
              format!("Content-Type: {}\r\n", content_type),
              encoding.map_or(String::new(), |enc| format!("Content-Transfer-Encoding: {}\r\n", enc.to_str())),
            ];
            
            for header in headers {
              if !header.is_empty() {
                data.extend_from_slice(header.as_bytes());
              }
            }
            
            data.extend_from_slice(b"\r\n");

            // Process file content based on encoding
            let mut content = file_data.into_bytes().await?;
            
            if let Some(enc) = encoding {
              content = match enc {
                ContentTransferEncoding::Base64 => {
                  base64::engine::general_purpose::STANDARD_NO_PAD
                    .encode(content)
                    .into_bytes()
                }
                ContentTransferEncoding::QuotedPrintable => {
                  quoted_printable::encode(&content)
                }
                ContentTransferEncoding::SevenBit | ContentTransferEncoding::EightBit => content,
              };
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
