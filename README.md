# http-client-multipart

[![Crates.io](https://img.shields.io/crates/v/http-client-multipart.svg)](https://crates.io/crates/http-client-multipart)
[![Docs.rs](https://docs.rs/http-client-multipart/badge.svg)](https://docs.rs/http-client-multipart)

This crate provides multipart request support for the [`http-client`](https://crates.io/crates/http-client) crate, enabling you to easily create and send multipart HTTP requests with file uploads and form data.  It is designed to be client-agnostic, working seamlessly with any `HttpClient` implementation.

## Features

*   **Client Agnostic:** Works with any HTTP client that implements the `http-client`'s `HttpClient` trait.
*   **File Uploads:** Easily add files to your multipart requests.
*   **Form Fields:** Include standard form fields in your requests.
*   **Correct Headers:** Automatically sets the `Content-Type` header with the correct boundary.
*   **Easy to Use:**  A simple and ergonomic API.

## Installation

Add `http-client-multipart` to your `Cargo.toml`:

```toml
[dependencies]
http-client-multipart = "0.1.0" # Replace with the latest version
http-client = "6.5.3" # Ensure you have the base http-client crate
async-std = "1.6.0" # Or tokio, or any async runtime you prefer
```

**Note:**  You also need to include a concrete implementation of the `http-client`'s `HttpClient` trait (if you haven't already).  Some popular choices:

*   `http-client` with `h1_client` (default, uses `async-h1`):
    ```toml
    http-client = { version = "6.5.3", features = ["h1_client", "native-tls"] }
    ```
*   `http-client` with `isahc` (`curl_client` feature):
    ```toml
    http-client = { version = "6.5.3", features = ["curl_client"] }
    ```
*   `http-client` with `hyper` (`hyper_client`feature):
    ```toml
    http-client = { version = "6.5.3", features = ["hyper_client"] }
    ```

## Usage

Here's a detailed guide on how to use the `http-client-multipart` crate:

### 1. Import the necessary crates and modules:

```rust
use http_client::{HttpClient, Request};
use http_types::{Method, Url};
use http_client_multipart::Multipart; // Import the Multipart struct
use http_client_multipart::RequestMultipartExt; // Import the extension trait (Optional)
use async_std::task;                          // Or tokio, or any async runtime you prefer
```

### 2. Create a `Multipart` instance:

```rust
let mut multipart = Multipart::new();
```

This creates a new multipart form with a randomly generated boundary. The boundary is used to separate each part of the multipart data.

### 3. Add fields to the `Multipart` form:

#### Adding Text Fields:

Use the `add_text` method to add standard form fields:

```rust
multipart.add_text("name", "John Doe");
multipart.add_text("age", "30");
```

#### Adding File Fields (from Path - Simplest Approach):

Use the `add_file` (async version) method to add files from their paths on the filesystem:

```rust
multipart.add_file("avatar", "path/to/your/image.jpg").await?; // requires an async context (.await)
```

This automatically infers the filename and content type based on the file's path.  It reads the file asynchronously, making it suitable for async contexts.  The `add_file` function needs to be awaited as it is an async function.

#### Adding File Fields (from Readers):

If you already have file data in memory or want more control over the filename and content type, you can add file fields using `add_async_read` or `add_sync_read`.

*   **`add_async_read` (Asynchronous Reader - recommended for async contexts):**

    ```rust
    use async_std::fs::File as AsyncFile;
    use async_std::io::BufReader;

    let file = AsyncFile::open("path/to/your/file.txt").await?;
    let buf_reader = BufReader::new(file); // Wrap the async file with a buffered reader
    multipart.add_async_read("document", "file.txt", "text/plain", buf_reader).await?;
    ```

    This method takes an asynchronous reader (`impl AsyncRead + Unpin + Send + 'static`) as input, along with the desired filename and content type. The data is read asynchronously into the body. The `add_async_read` function needs to be awaited as it is an async function.

*   **`add_sync_read` (Synchronous Reader - use with caution in async contexts):**

    ```rust
    use std::fs::File;

    let file = File::open("path/to/your/file.txt")?;
    multipart.add_sync_read("config", "config.txt", "text/plain", file)?;
    ```

    This method takes a synchronous reader (`impl Read + Seek + Send + 'static`) as input, along with the desired filename and content type. Use this in synchronous contexts, or if you are very careful about thread blocking in async.

*   **`add_file_from_sync` (Convenience for `File` objects - synchronous):**

    ```rust
    use std::fs::File;

    let file = File::open("path/to/your/file.txt")?;
    multipart.add_file_from_sync("archive", "data.zip", "application/zip", file)?;
    ```
    This is a shorthand for creating a `File` object directly and adding it.

### 4. Create an `http-client` `Request`:

```rust
let url = "https://httpbin.org/post".parse::<Url>()?; // Replace with your API endpoint
let mut req = Request::new(Method::Post, url);
```

### 5. Set the `Multipart` data as the request body:

There are two ways to do this:

#### Method 1: Using `set_request(req: &mut Request)` (Mutates Existing Request - Preferred):

This is the **recommended** approach because it encapsulates all the logic within the `Multipart` struct:

```rust
multipart.set_request(&mut req)?;
```

This method will:

*   Convert the `Multipart` form into a `Body`.
*   Set the `Content-Type` header of the request to `multipart/form-data` with the correct boundary.
*   Set the body of the request to the converted body.

#### Method 2: Using the `RequestMultipartExt` trait (Extension Method):

This approach adds an extension method to the  `Request` object:

```rust
use http_client_multipart::RequestMultipartExt;

req.set_multipart_body(multipart)?;
```

Both achieve the same outcome, but `set_request` offers better encapsulation.

### 6. Create an `HttpClient` and send the request:

```rust
use http_client::h1::H1Client as Client;  // Example: Using h1_client

let client = Client::new();

let mut response = client.send(req).await?;
```

Remember to choose a concrete `HttpClient` implementation based on your needs (e.g., `H1Client`, `IsahcClient`, `HyperClient`).

### 7. Handle the response:

```rust
let body = response.body_string().await?;

println!("{}", body);
```

## Complete Example (using async-std and h1_client):

```rust
use http_client::{HttpClient, Request};
use http_types::{Method, Url, Result};
use http_client_multipart::Multipart;
use http_client_multipart::RequestMultipartExt;
use async_std::task;

use http_client::h1::H1Client as Client;

async fn send_multipart_request() -> Result<()> {
    // 1. Create a Multipart instance
    let mut multipart = Multipart::new();

    // 2. Add fields to the Multipart form
    multipart.add_text("name", "John Doe");
    multipart.add_text("age", "30");

    // Add a file (assuming you have a file named "image.jpg" in the same directory)
    multipart.add_file("avatar", "Cargo.toml").await?;

    // 3. Create an http-client Request
    let url = "https://httpbin.org/post".parse::<Url>()?; // Replace with your API endpoint
    let mut req = Request::new(Method::Post, url);

    // 4. Set the Multipart data as the request body using set_request()
    multipart.set_request(&mut req)?;

    // 5. Create an HttpClient and send the request
  
    let client = Client::new(); // Or any other HttpClient implementation

  
    let mut response = client.send(req).await?;

    // 6. Handle the response
  
    let body = response.body_string().await?;

  
    println!("{}", body);

    Ok(())
}

fn main() -> Result<()> {
    task::block_on(async {
        send_multipart_request().await
    })
}
```

## Notes

*   Error Handling:  The examples above use `?` for error propagation. In a real application, handle errors gracefully.
*   File Paths: Ensure that the file paths you provide to `add_file` are correct and accessible.
*   Content Types:  While automatic content type detection is provided, you might need to specify the content type explicitly for certain file types using  `add_async_read` or `add_sync_read` if the automatic detection is inaccurate.
*   Performance:   For very large files, consider streaming the file data instead of reading it all into memory at once, using `add_async_read`.

## License

This crate is licensed under the [MIT License](LICENSE.md).

