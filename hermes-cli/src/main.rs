use anyhow::Result;
use clap::{Parser, Subcommand};
use hermes_core::{
    crypto::CryptoService,
    NetworkService,
};
use libp2p::PeerId;
use std::str::FromStr;
use log::info;

#[derive(Parser)]
#[clap(name = "hermes", version = "0.1.0", about = "Project Hermes secure messaging CLI")]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the Hermes node
    Start {
        /// Listen on this address
        #[clap(long, default_value = "127.0.0.1:8080")]
        listen: String,
    },
    /// Send a message to a peer
    Send {
        /// Peer ID to send the message to
        #[clap(long)]
        peer: String,
        /// Message to send
        #[clap(long)]
        message: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();

    info!("Project Hermes CLI - Secure Messaging Backend");
    
    let cli = Cli::parse();
    
    // Initialize the core components
    let crypto_service = CryptoService::new()?;
    let mut network = NetworkService::new(crypto_service).await?;
    
    // Process command
    match cli.command {
        Command::Start { listen } => {
            info!("Starting Hermes node on {}", listen);
            
            // Display the local peer ID
            let peer_id = network.get_peer_id();
            println!("Your Peer ID: {}", peer_id.to_base58());
            info!("Your Peer ID: {}", peer_id.to_base58());
            
            // Set up a listener on the specified address
            let addr: std::net::SocketAddr = listen.parse().expect("Failed to parse listen address");
            let ip_component = match addr.ip() {
                std::net::IpAddr::V4(ip) => libp2p::Multiaddr::from(ip),
                std::net::IpAddr::V6(ip) => libp2p::Multiaddr::from(ip),
            };
            let listen_addr = ip_component.with(libp2p::multiaddr::Protocol::Tcp(addr.port()));
            
            // Listen on the specified address
            let _ = network.swarm_mut().listen_on(listen_addr)
                .expect("Failed to listen on address");
            
            // Register as relay if enabled
            network.register_as_relay().await?;
            
            // Start the network service - this will block until finished
            network.start().await?;
        },
        Command::Send { peer, message } => {
            info!("Sending message to peer {}", peer);
            
            // Parse peer ID
            let peer_id = match PeerId::from_str(&peer) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("Error parsing peer ID: {}", e);
                    return Err(anyhow::anyhow!("Invalid peer ID format"));
                }
            };
            
            // Convert message string to bytes
            let message_bytes = message.as_bytes().to_vec();
            
            // Send message
            network.send_message(peer_id, message_bytes).await?;
            info!("Message sent successfully");
        }
    }

    Ok(())
}
