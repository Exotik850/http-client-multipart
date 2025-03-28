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
//! multipart.add_file("avatar", "examples/avatar.jpg", None).await?;
//!
//! // Create a request.
//! let url = "https://httpbin.org/post".parse::<Url>()?;
//! let mut req = Request::new(Method::Post, url);
//!
//! // Set the multipart body.
//! multipart.set_request(&mut req);
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
use http_types::Request;

mod encoding;
mod multipart;
mod part;
mod reader_stream;

pub use encoding::Encoding;
pub use multipart::Multipart;

pub type StreamChunk = std::result::Result<Vec<u8>, futures_lite::io::Error>;

/// Generates a random boundary string.
fn generate_boundary() -> String {
    (0..30).map(|_| fastrand::alphanumeric()).collect()
}

// Extension trait for adding multipart functionality.
pub trait RequestMultipartExt {
    fn multipart(&mut self, multipart: Multipart);
}

impl RequestMultipartExt for Request {
    fn multipart(&mut self, multipart: Multipart) {
        multipart.set_request(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::multipart::Multipart;

    use super::*;
    use http_types::{Method, Result, Url};

    #[async_std::test]
    async fn test_multipart_text() -> Result<()> {
        let mut multipart = Multipart::new();
        multipart.add_text("name", "John Doe");
        multipart.add_text("age", "42");

        let mut req = Request::new(Method::Post, Url::parse("http://example.com")?);
        multipart.set_request(&mut req);

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
        multipart.set_request(&mut req);

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
        multipart.set_request(&mut req);

        let content_type = req.header("Content-Type").unwrap().last().as_str();
        assert!(content_type.starts_with("multipart/form-data; boundary="));

        let body = dbg!(req.body_string().await?);
        assert!(body.contains("John Doe"));
        assert!(body.contains("[package]"));
        Ok(())
    }

    // #[async_std::test]
    // async fn example_test() -> Result<()> {
    //     use http_client::h1::H1Client as Client;
    //     use http_client::HttpClient;

    //     // Create a new multipart form.
    //     let mut multipart = Multipart::new();

    //     // Add a text field.
    //     multipart.add_text("name", "John Doe");

    //     // Add a file.
    //     multipart.add_file("avatar", "Cargo.toml", None).await?;

    //     // Create a request.
    //     let url = "https://httpbin.org/post".parse::<Url>()?;
    //     let mut req = Request::new(Method::Post, url);

    //     // Set the multipart body.
    //     multipart.set_request(&mut req);

    //     // Create a client.
    //     let client = Client::new();

    //     // Send the request.
    //     let mut response = client.send(req).await?;

    //     // Read the response body.
    //     let body = response.body_string().await?;

    //     // Print the response body.
    //     println!("{}", body);

    //     Ok(())
    // }
}
