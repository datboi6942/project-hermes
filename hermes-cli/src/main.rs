use anyhow::Result;
use clap::{Parser, Subcommand};
use hermes_core::{
    crypto::CryptoService,
    NetworkService,
};
use std::net::SocketAddr;
use log::{info, warn};

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
        
        /// Use Tor for enhanced privacy (requires Tor daemon running)
        #[clap(long)]
        use_tor: bool,
    },
    /// Connect to a peer
    Connect {
        /// Peer ID to connect to
        #[clap(long)]
        peer: String,
        /// Address of the peer (e.g. "192.168.1.5:9000")
        #[clap(long)]
        address: String,
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
    /// Connect to a peer and send a message in one operation
    Direct {
        /// Peer ID to connect to and send message
        #[clap(long)]
        peer: String,
        /// Address of the peer (e.g. "192.168.1.5:9000")
        #[clap(long)]
        address: String,
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
        Command::Start { listen, use_tor } => {
            info!("Starting Hermes node on {}", listen);
            
            // Display the local node ID
            let node_id = network.get_node_id();
            println!("Your Node ID: {}", node_id);
            info!("Your Node ID: {}", node_id);
            
            // Set up Tor if requested
            if use_tor {
                println!("Setting up Tor routing for enhanced privacy...");
                if let Err(e) = network.setup_tor_transport().await {
                    warn!("Failed to set up Tor transport: {}", e);
                    println!("WARNING: Tor setup failed. Continuing with standard networking.");
                } else {
                    println!("Tor routing enabled. Your IP address will be hidden.");
                }
            } else {
                println!("TIP: For better privacy, restart with --use-tor flag to route traffic through Tor.");
            }
            
            // Set up a listener on the specified address
            let addr: SocketAddr = listen.parse().expect("Failed to parse listen address");
            
            println!("Node is running. Messages will be received automatically.");
            println!("Press Ctrl+C to exit.");
            
            // Start the network service - this will block until finished
            network.start(addr).await?;
        },
        Command::Connect { peer, address } => {
            info!("Connecting to peer {} at {}", peer, address);
            
            // Parse socket address
            let addr: SocketAddr = address.parse().expect("Failed to parse socket address");
            
            // Connect to the peer
            network.connect_to_peer(peer.clone(), addr).await?;
            
            println!("Successfully connected to peer {}", peer);
            info!("Connected to peer {}", peer);
        },
        Command::Send { peer, message } => {
            info!("Sending message to peer {}", peer);
            
            // Convert message string to bytes
            let message_bytes = message.as_bytes().to_vec();
            
            // Send message
            match network.send_message(peer.clone(), message_bytes).await {
                Ok(_) => {
                    println!("Message sent successfully to {}", peer);
                },
                Err(e) => {
                    println!("Failed to send message: {}", e);
                    println!("Note: The recipient must be online and connected.");
                    println!("Try running 'connect' command first to establish a connection.");
                    return Err(e);
                }
            }
        },
        Command::Direct { peer, address, message } => {
            info!("Connecting to peer {} at {} and sending message", peer, address);
            
            // Parse socket address
            let addr: SocketAddr = address.parse().expect("Failed to parse socket address");
            
            // Connect to the peer
            network.connect_to_peer(peer.clone(), addr).await?;
            
            // Convert message string to bytes
            let message_bytes = message.as_bytes().to_vec();
            
            // Send message (with a short delay to ensure connection is established)
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            
            match network.send_message(peer.clone(), message_bytes).await {
                Ok(_) => {
                    println!("Successfully connected and sent message to {}", peer);
                },
                Err(e) => {
                    println!("Connected to peer but failed to send message: {}", e);
                    return Err(e);
                }
            }
        },
    }

    Ok(())
}
