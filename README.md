# Project Hermes

A secure, cross-platform messaging application with end-to-end encryption and IP obfuscation.

## Overview

Project Hermes is a minimalistic, secure messaging application that prioritizes privacy and performance. Built with a Rust backend for critical security operations and cross-platform support for Android and desktop clients.

### Key Features

- End-to-end encrypted messaging
- Peer-to-peer communication (serverless)
- IP address obfuscation using onion routing
- No-logs policy
- Minimalistic UI
- High-performance Rust backend

## Project Structure

```
hermes-core/           # Rust backend implementation
├── src/
│   ├── crypto/       # Cryptographic operations
│   ├── protocol/     # Messaging protocol implementation
│   ├── onion.rs      # Onion routing implementation
│   ├── discovery.rs  # Node discovery service
│   └── utils/        # Utility functions
└── Cargo.toml        # Rust dependencies and configuration

hermes-cli/            # Command-line interface
├── src/
│   └── main.rs       # CLI implementation
└── Cargo.toml        # CLI dependencies

hermes-android/        # Android client (TODO)
hermes-desktop/        # Desktop client (TODO)
```

## Building

### Prerequisites

- Rust (latest stable)
- Android SDK (for future Android development)
- Flutter/Dart (for future desktop client)

### Building the Project

```bash
# Build both core and CLI components
cargo build --release

# Run the CLI
cargo run --bin hermes-cli -- --help
```

### Running the Node

```bash
# Start a node
cargo run --bin hermes-cli -- start --listen 127.0.0.1:9000
```

This will display your Peer ID, which other users will need to send messages to you:

```
Your Peer ID: 12D3KooWA8EXV2fkLsSXPzBt48wCaZ7tMWNiBkawChQBqKyqAkDL
```

# Send a message
cargo run --bin hermes-cli -- send --peer <PEER_ID> --message "Hello, world!"
```

## Security Features

- End-to-end encryption using modern cryptographic standards (ed25519-dalek and x25519-dalek)
- Secure key exchange protocols
- Circuit-based onion routing for IP obfuscation
- Node discovery mechanism for finding peers
- Memory-safe implementation in Rust

## Development Status

Currently in early development. Core Rust backend and CLI interface are functional with basic messaging capabilities.

## License

MIT License - See LICENSE file for details 