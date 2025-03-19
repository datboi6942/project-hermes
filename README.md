# Project Hermes

A secure, cross-platform messaging application with end-to-end encryption and IP obfuscation.

## Overview

Project Hermes is a minimalistic, secure messaging application that prioritizes privacy and performance. Built with a Rust backend for critical security operations and cross-platform support for Android and desktop clients.

### Key Features

- End-to-end encrypted messaging
- Peer-to-peer communication (serverless)
- IP address obfuscation
- No-logs policy
- Minimalistic UI
- High-performance Rust backend

## Project Structure

```
hermes-core/           # Rust backend implementation
├── src/
│   ├── crypto/       # Cryptographic operations
│   ├── network/      # P2P networking and routing
│   ├── protocol/     # Messaging protocol implementation
│   └── utils/        # Utility functions
├── tests/            # Integration and unit tests
└── Cargo.toml        # Rust dependencies and configuration

hermes-android/       # Android client (TODO)
hermes-desktop/       # Desktop client (TODO)
```

## Building

### Prerequisites

- Rust (latest stable)
- Android SDK (for Android development)
- Flutter/Dart (for desktop client)

### Building the Rust Backend

```bash
cd hermes-core
cargo build --release
```

## Security Features

- End-to-end encryption using modern cryptographic standards
- Secure key exchange protocols
- Ephemeral messaging sessions
- No persistent logging
- IP address obfuscation through onion routing
- Memory-safe implementation in Rust

## Development Status

Currently in early development. Core Rust backend implementation is in progress.

## License

MIT License - See LICENSE file for details 