use async_std::{io::WriteExt, stream::StreamExt};
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

    let mut reader = multipart.into_stream(Some(32));
    let mut stdout = async_std::io::stdout();

    while let Some(chunk) = reader.next().await {
        let chunk = chunk?;
        stdout.write_all(&chunk).await?;
        stdout.flush().await?; // Ensure the output is flushed after each chunk
                               // sleep
        async_std::task::sleep(std::time::Duration::from_millis(75)).await;
    }

    Ok(())
}
