use async_std::stream::StreamExt;
use http_client_multipart::{Encoding, Multipart};

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut multipart = Multipart::new();

    multipart.add_text("field1", "value1");
    multipart.add_text("field2", "value2");

    multipart.add_file("file_1", "./examples/file.txt", None).await?;
    multipart
        .add_file(
            "file_2",
            "./LICENSE.md",
            Some(Encoding::Base64),
        )
        .await?;

    let mut stream = multipart.into_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        // Here you can process the chunk, e.g., write it to a file or send it over a network
        println!("Chunk: {:?}", String::from_utf8_lossy(&chunk));

        async_std::task::sleep(std::time::Duration::from_millis(100)).await; // Simulate some processing delay
    }

    Ok(())
}
