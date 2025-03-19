#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}Setting up Hermes on Raspberry Pi...${NC}"

# Check if running on Raspberry Pi
if ! grep -q "Raspberry Pi" /proc/cpuinfo; then
    echo -e "${RED}This script should only be run on a Raspberry Pi${NC}"
    exit 1
fi

# Install required packages
echo -e "${BLUE}Installing required packages...${NC}"
sudo apt-get update
sudo apt-get install -y build-essential git curl pkg-config libssl-dev

# Install Rust if not already installed
if ! command -v rustc &> /dev/null; then
    echo -e "${BLUE}Installing Rust...${NC}"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
fi

# Clone the repository
echo -e "${BLUE}Cloning Hermes repository...${NC}"
cd ~
if [ ! -d "Project_Hermes" ]; then
    git clone https://github.com/john/Project_Hermes.git
    cd Project_Hermes
else
    cd Project_Hermes
    git pull
fi

# Build the project
echo -e "${BLUE}Building Hermes...${NC}"
cargo build --release

# Create necessary directories
mkdir -p test_files
mkdir -p test_output

# Get IP address
IP_ADDRESS=$(hostname -I | awk '{print $1}')
echo -e "${GREEN}Setup completed successfully!${NC}"
echo -e "${BLUE}Your Raspberry Pi's IP address is: ${IP_ADDRESS}${NC}"
echo -e "${YELLOW}Please note this IP address for connecting from your main machine${NC}" 