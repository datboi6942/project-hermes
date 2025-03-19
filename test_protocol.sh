#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to check if running on Raspberry Pi
is_raspberry_pi() {
    if [ -f /proc/cpuinfo ]; then
        if grep -q "Raspberry Pi" /proc/cpuinfo; then
            return 0
        fi
    fi
    return 1
}

# Function to get IP address
get_ip_address() {
    if is_raspberry_pi; then
        hostname -I | awk '{print $1}'
    else
        ip route get 1 | awk '{print $7;exit}'
    fi
}

# Function to check if required tools are installed
check_requirements() {
    local missing=()
    
    # Check for Rust
    if ! command -v rustc &> /dev/null; then
        missing+=("rustc")
    fi
    
    # Check for cargo
    if ! command -v cargo &> /dev/null; then
        missing+=("cargo")
    fi
    
    # Check for git
    if ! command -v git &> /dev/null; then
        missing+=("git")
    fi
    
    if [ ${#missing[@]} -ne 0 ]; then
        echo -e "${RED}Missing required tools: ${missing[*]}${NC}"
        echo -e "${YELLOW}Installing required tools...${NC}"
        
        if is_raspberry_pi; then
            sudo apt-get update
            sudo apt-get install -y build-essential git curl
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source $HOME/.cargo/env
        else
            # Add instructions for other Linux distributions if needed
            echo -e "${RED}Please install the missing tools manually${NC}"
            exit 1
        fi
    fi
}

# Function to setup SSH access
setup_ssh() {
    if ! is_raspberry_pi; then
        echo -e "${BLUE}Setting up SSH access to Raspberry Pi...${NC}"
        
        # Check if SSH key exists
        if [ ! -f ~/.ssh/id_rsa ]; then
            ssh-keygen -t rsa -b 4096 -f ~/.ssh/id_rsa -N ""
        fi
        
        # Copy SSH key to Raspberry Pi
        echo -e "${YELLOW}Please enter the Raspberry Pi's IP address:${NC}"
        read pi_ip
        
        echo -e "${YELLOW}Please enter the Raspberry Pi's username (default: pi):${NC}"
        read pi_user
        pi_user=${pi_user:-pi}
        
        ssh-copy-id $pi_user@$pi_ip
    fi
}

# Function to clone and build the project
setup_project() {
    echo -e "${BLUE}Setting up Hermes project...${NC}"
    
    # Clone the repository if not already cloned
    if [ ! -d "Project_Hermes" ]; then
        git clone https://github.com/yourusername/Project_Hermes.git
        cd Project_Hermes
    else
        cd Project_Hermes
    fi
    
    # Build the project
    cargo build --release
    
    # Create necessary directories
    mkdir -p test_files
    mkdir -p test_output
}

# Function to simulate network conditions
simulate_network() {
    local latency=$1
    local bandwidth=$2
    local packet_loss=$3
    
    echo -e "${BLUE}Simulating network conditions:${NC}"
    echo "Latency: ${latency}ms"
    echo "Bandwidth: ${bandwidth}MB/s"
    echo "Packet Loss: ${packet_loss}%"
    
    # Clean up existing rules
    sudo tc qdisc del dev lo root 2>/dev/null || true
    
    # Add new rules
    sudo tc qdisc add dev lo root netem delay ${latency}ms ${latency}ms distribution normal loss ${packet_loss}% rate ${bandwidth}mbps
}

# Function to run transfer test
run_transfer_test() {
    local file=$1
    local priority=$2
    
    echo -e "${BLUE}Testing file transfer: ${file}${NC}"
    echo "Priority: ${priority}"
    
    # Start the transfer
    cargo run --release --bin hermes-cli -- transfer --file "$file" --priority "$priority" &
    local transfer_pid=$!
    
    # Monitor progress
    while kill -0 $transfer_pid 2>/dev/null; do
        echo -n "."
        sleep 1
    done
    
    # Check if transfer completed successfully
    if wait $transfer_pid; then
        echo -e "\n${GREEN}Transfer completed successfully${NC}"
    else
        echo -e "\n${RED}Transfer failed${NC}"
    fi
}

# Function to test communication between devices
test_communication() {
    if ! is_raspberry_pi; then
        echo -e "${BLUE}Testing communication with Raspberry Pi...${NC}"
        
        # Get Raspberry Pi's IP address
        echo -e "${YELLOW}Please enter the Raspberry Pi's IP address:${NC}"
        read pi_ip
        
        # Create a test file
        dd if=/dev/urandom of=test_files/test.txt bs=1M count=1
        
        # Start the receiver on Raspberry Pi
        echo -e "${BLUE}Starting receiver on Raspberry Pi...${NC}"
        ssh pi@$pi_ip "cd Project_Hermes && cargo run --release --bin hermes-cli -- receive" &
        
        # Wait for receiver to start
        sleep 2
        
        # Send the test file
        echo -e "${BLUE}Sending test file to Raspberry Pi...${NC}"
        cargo run --release --bin hermes-cli -- transfer --file test_files/test.txt --priority 1
        
        # Wait for transfer to complete
        wait
        
        echo -e "${GREEN}Communication test completed${NC}"
    fi
}

# Main setup process
echo -e "${BLUE}Starting Hermes setup...${NC}"

# Check requirements
check_requirements

# Setup SSH if not on Raspberry Pi
setup_ssh

# Setup project
setup_project

# Test communication
test_communication

echo -e "${GREEN}Setup completed successfully${NC}" 