use async_std::stream::StreamExt;
use futures_lite::AsyncReadExt;
use http_client_multipart::{Encoding, Multipart};

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut multipart = Multipart::new();

    multipart.add_text("field1", "value1");
    multipart.add_text("field2", "value2");

    multipart
        .add_file("file_1", "./examples/file.txt", None)
        .await?;
    multipart
        .add_file("file_2", "./LICENSE.md", Some(Encoding::Base64))
        .await?;

    let mut reader = multipart.into_reader();

    let mut out = String::new();
    reader.read_to_string(&mut out).await?;

    println!("{}", out);

    Ok(())
}
