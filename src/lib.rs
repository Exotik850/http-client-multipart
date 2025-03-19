//! Multipart request support for the `http-client` crate.
//!
//! This module provides functionality to create and send multipart requests using the `http-client` crate.
//! It supports file uploads, form fields, and custom headers for each part.
//!
//! Example:
//!
//! ```rust
//! # #[cfg(any(feature = "h1_client", feature = "docs"))]
//! # use http_client::h1::H1Client as Client;
//! # use http_client::{HttpClient, Request};
//! # use http_types::{Method, Url};
//! # use http_client_multipart::Multipart;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Create a new multipart form.
//! let mut multipart = Multipart::new();
//!
//! // Add a text field.
//! multipart.add_text("name", "John Doe");
//!
//! // Add a file.
//! multipart.add_file("avatar", "examples/avatar.jpg").await?;
//!
//! // Create a request.
//! let url = "https://httpbin.org/post".parse::<Url>()?;
//! let mut req = Request::new(Method::Post, url);
//!
//! // Set the multipart body.
//! multipart.set_request(&mut req).await?;
//!
//! // Create a client.
//! # #[cfg(any(feature = "h1_client", feature = "docs"))]
//! let client = Client::new();
//!
//! // Send the request.
//! # #[cfg(any(feature = "h1_client", feature = "docs"))]
//! let mut response = client.send(req).await?;
//!
//! // Read the response body.
//! # #[cfg(any(feature = "h1_client", feature = "docs"))]
//! let body = response.body_string().await?;
//!
//! // Print the response body.
//! # #[cfg(any(feature = "h1_client", feature = "docs"))]
//! println!("{}", body);
//!
//! # Ok(())
//! # }
//! ```
use async_fs::File as AsyncFile;
use base64::Engine;
use futures_lite::io::BufReader;
use futures_lite::AsyncBufRead;
use http_types::{Body, Request, Result};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

/// A struct representing a multipart form.
#[derive(Debug)]
pub struct Multipart {
    boundary: String,
    fields: Vec<Field>,
}

/// Represents a single field in a multipart form.
#[derive(Debug)]
enum Field {
    /// A text field.
    Text { name: String, value: String },
    /// A file field.
    File {
        name: String,
        filename: String,
        content_type: String,
        encoding: Option<ContentTransferEncoding>,
        data: Body,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    Base64,
    QuotedPrintable,
}

impl ContentTransferEncoding {
    fn to_str(&self) -> &'static str {
        match self {
            ContentTransferEncoding::SevenBit => "7bit",
            ContentTransferEncoding::EightBit => "8bit",
            ContentTransferEncoding::Base64 => "base64",
            ContentTransferEncoding::QuotedPrintable => "quoted-printable",
        }
    }
}

impl Default for Multipart {
    fn default() -> Self {
        Self::new()
    }
}

impl Multipart {
    /// Creates a new `Multipart` form with a randomly generated boundary.
    pub fn new() -> Self {
        Self {
            boundary: generate_boundary(),
            fields: Vec::new(),
        }
    }

    /// Adds a text field to the form.
    pub fn add_text(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.fields.push(Field::Text {
            name: name.into(),
            value: value.into(),
        });
    }

    /// Adds a file field to the form from path.
    pub async fn add_file(
        &mut self,
        name: impl Into<String>,
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
        name: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        encoding: Option<ContentTransferEncoding>,
        data: impl AsyncBufRead + Unpin + Send + Sync + 'static,
    ) -> Result<()> {
        let body = Body::from_reader(data, None);
        self.fields.push(Field::File {
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
        name: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        encoding: Option<ContentTransferEncoding>,
        mut data: impl Read + Seek + Send + 'static,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        data.read_to_end(&mut buffer)?;
        let body = Body::from_bytes(buffer);
        self.fields.push(Field::File {
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
        name: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        encoding: Option<ContentTransferEncoding>,
        file: File,
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
            match field {
                Field::Text { name, value } => {
                    data.extend_from_slice(b"--");
                    data.extend_from_slice(self.boundary.as_bytes());
                    data.extend_from_slice(b"\r\n");
                    data.extend_from_slice(
                        format!("Content-Disposition: form-data; name=\"{}\"\r\n", name).as_bytes(),
                    );
                    data.extend_from_slice(b"\r\n");
                    data.extend(value.into_bytes());
                    data.extend_from_slice(b"\r\n");
                }
                Field::File {
                    name,
                    filename,
                    content_type,
                    encoding,
                    data: d,
                } => {
                    data.extend_from_slice(b"--");
                    data.extend_from_slice(self.boundary.as_bytes());
                    data.extend_from_slice(b"\r\n");
                    data.extend_from_slice(
                        format!(
                            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                            name, filename
                        )
                        .as_bytes(),
                    );
                    data.extend_from_slice(
                        format!("Content-Type: {}\r\n", content_type).as_bytes(),
                    );
                    if let Some(enc) = encoding {
                        data.extend_from_slice(
                            format!("Content-Transfer-Encoding: {}\r\n", enc.to_str()).as_bytes(),
                        );
                    }
                    data.extend_from_slice(b"\r\n");
                    let mut b = d.into_bytes().await?;
                    if let Some(enc) = encoding {
                        match enc {
                            ContentTransferEncoding::Base64 => {
                                // b = base64::encode(&b).into_bytes();
                                b = base64::engine::general_purpose::STANDARD_NO_PAD
                                    .encode(b)
                                    .into_bytes();
                            }
                            ContentTransferEncoding::QuotedPrintable => {
                                b = quoted_printable::encode(&b);
                            }
                            _ => {}
                        }
                    }
                    data.extend(b);
                    data.extend_from_slice(b"\r\n");
                }
            }
        }

        // Close boundary
        data.extend_from_slice(b"--");
        data.extend_from_slice(self.boundary.as_bytes());
        data.extend_from_slice(b"--\r\n");

        Ok(Body::from(data))
    }
}

/// Generates a random boundary string.
fn generate_boundary() -> String {
    (0..30).map(|_| fastrand::alphanumeric()).collect()
}

// Extension trait for adding multipart functionality.
pub trait RequestMultipartExt {
    fn set_multipart_body(
        &mut self,
        multipart: Multipart,
    ) -> impl std::future::Future<Output = Result<()>>;
}

impl RequestMultipartExt for Request {
    fn set_multipart_body(
        &mut self,
        multipart: Multipart,
    ) -> impl std::future::Future<Output = Result<()>> {
        multipart.set_request(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_types::{Method, Url};

    #[async_std::test]
    async fn test_multipart_text() -> Result<()> {
        let mut multipart = Multipart::new();
        multipart.add_text("name", "John Doe");
        multipart.add_text("age", "42");

        let mut req = Request::new(Method::Post, Url::parse("http://example.com")?);
        multipart.set_request(&mut req).await?;

        let content_type = req.header("Content-Type").unwrap().last().as_str();
        assert!(content_type.starts_with("multipart/form-data; boundary="));

        let body = req.body_string().await?;
        assert!(body.contains("John Doe"));
        assert!(body.contains("42"));
        Ok(())
    }

    #[async_std::test]
    async fn test_multipart_file() -> Result<()> {
        let mut multipart = Multipart::new();
        multipart.add_file("avatar", "Cargo.toml", None).await?;

        let mut req = Request::new(Method::Post, Url::parse("http://example.com")?);
        multipart.set_request(&mut req).await?;

        let content_type = req.header("Content-Type").unwrap().last().as_str();
        assert!(content_type.starts_with("multipart/form-data; boundary="));

        let body = req.body_string().await?;
        assert!(body.contains("[package]"));
        Ok(())
    }

    #[async_std::test]
    async fn test_multipart_mixed() -> Result<()> {
        let mut multipart = Multipart::new();
        multipart.add_text("name", "John Doe");
        multipart.add_file("avatar", "Cargo.toml", None).await?;

        let mut req = Request::new(Method::Post, Url::parse("http://example.com")?);
        multipart.set_request(&mut req).await?;

        let content_type = req.header("Content-Type").unwrap().last().as_str();
        assert!(content_type.starts_with("multipart/form-data; boundary="));

        let body = dbg!(req.body_string().await?);
        assert!(body.contains("John Doe"));
        assert!(body.contains("[package]"));
        Ok(())
    }

    #[async_std::test]
    async fn example_test() -> Result<()> {
        use http_client::h1::H1Client as Client;
        use http_client::HttpClient;

        // Create a new multipart form.
        let mut multipart = Multipart::new();

        // Add a text field.
        multipart.add_text("name", "John Doe");

        // Add a file.
        multipart.add_file("avatar", "Cargo.toml", None).await?;

        // Create a request.
        let url = "https://httpbin.org/post".parse::<Url>()?;
        let mut req = Request::new(Method::Post, url);

        // Set the multipart body.
        multipart.set_request(&mut req).await?;

        // Create a client.
        let client = Client::new();

        // Send the request.
        let mut response = client.send(req).await?;

        // Read the response body.
        let body = response.body_string().await?;

        // Print the response body.
        println!("{}", body);

        Ok(())
    }
}
