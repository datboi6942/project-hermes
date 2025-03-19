use clap::{App, Arg, SubCommand};
use std::path::PathBuf;
use std::time::Duration;
use hermes_core::network::protocol::ProtocolHandler;
use hermes_core::crypto::CryptoService;
use anyhow::Result;
use std::io::{self, Write, Read, BufReader};
use std::fs::File;

#[tokio::main]
async fn main() -> Result<()> {
    let matches = App::new("Hermes CLI")
        .version("1.0")
        .author("Hermes Team")
        .about("Command line interface for Hermes protocol")
        .subcommand(
            SubCommand::with_name("transfer")
                .about("Transfer a file using the Hermes protocol")
                .arg(
                    Arg::with_name("file")
                        .help("Path to the file to transfer")
                        .required(true)
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("priority")
                        .help("Transfer priority (0-4)")
                        .required(false)
                        .takes_value(true)
                        .default_value("0"),
                ),
        )
        .subcommand(
            SubCommand::with_name("receive")
                .about("Receive files using the Hermes protocol")
                .arg(
                    Arg::with_name("output_dir")
                        .help("Directory to save received files")
                        .required(false)
                        .takes_value(true)
                        .default_value("test_output"),
                ),
        )
        .get_matches();

    // Initialize protocol handler
    let crypto_service = CryptoService::new()?;
    let mut handler = ProtocolHandler::new(crypto_service);

    if let Some(matches) = matches.subcommand_matches("transfer") {
        let file_path = matches.value_of("file").unwrap();
        let priority: u8 = matches.value_of("priority").unwrap().parse()?;

        // Read file
        let file_path = PathBuf::from(file_path);
        let file_size = std::fs::metadata(&file_path)?.len();
        let file_name = file_path.file_name().unwrap().to_string_lossy().to_string();

        println!("Starting transfer of {} ({} bytes)", file_name, file_size);
        println!("Priority: {}", priority);

        // Start transfer
        let file_id = handler.generate_ephemeral_id();
        let mut bytes_transferred = 0;
        let chunk_size = 64 * 1024; // 64KB chunks

        let file = std::fs::File::open(&file_path)?;
        let mut reader = std::io::BufReader::new(file);
        let mut buffer = vec![0; chunk_size];

        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }

            let chunk = &buffer[..n];
            handler.send_file_chunk(
                file_id.clone(),
                chunk.to_vec(),
                bytes_transferred,
                file_size,
                priority,
            ).await?;

            bytes_transferred += n as u64;
            let progress = (bytes_transferred as f64 / file_size as f64) * 100.0;
            print!("\rProgress: {:.1}%", progress);
            std::io::stdout().flush()?;

            // Small delay to prevent overwhelming the network
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        println!("\nTransfer completed successfully!");
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let output_dir = matches.value_of("output_dir").unwrap();
        std::fs::create_dir_all(output_dir)?;

        println!("Starting file receiver...");
        println!("Saving files to: {}", output_dir);

        // Start receiving files
        loop {
            match handler.receive_file_chunk().await {
                Ok((file_id, chunk, offset, total_size, _)) => {
                    let file_path = PathBuf::from(output_dir).join(format!("{}.part", file_id));
                    let mut file = if offset == 0 {
                        File::create(&file_path)?
                    } else {
                        File::options().append(true).open(&file_path)?
                    };

                    file.write_all(&chunk)?;

                    let progress = ((offset + chunk.len() as u64) as f64 / total_size as f64) * 100.0;
                    print!("\rReceiving file {}: {:.1}%", file_id, progress);
                    std::io::stdout().flush()?;

                    // If this is the last chunk, rename the file
                    if offset + chunk.len() as u64 >= total_size {
                        let final_path = PathBuf::from(output_dir).join(file_id);
                        std::fs::rename(file_path, final_path)?;
                        println!("\nFile {} received successfully!", file_id);
                    }
                }
                Err(e) => {
                    println!("\nError receiving file: {}", e);
                }
            }
        }
    }

    Ok(())
} 