#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to check if tc is available
check_tc() {
    if ! command -v tc &> /dev/null; then
        echo -e "${RED}Error: tc command not found. Please install iproute2 package.${NC}"
        exit 1
    fi
}

# Function to cleanup network settings
cleanup_network() {
    echo -e "${BLUE}Cleaning up network settings...${NC}"
    sudo tc qdisc del dev lo root 2>/dev/null || true
}

# Set up cleanup on script exit
trap cleanup_network EXIT

# Check for required tools
check_tc

echo -e "${BLUE}Starting Local Protocol Test Suite${NC}"

# Create test directories
mkdir -p test_files
mkdir -p test_output

# Generate test files of different sizes
echo -e "${BLUE}Generating test files...${NC}"
dd if=/dev/urandom of=test_files/small.txt bs=1K count=1
dd if=/dev/urandom of=test_files/medium.txt bs=1M count=1
dd if=/dev/urandom of=test_files/large.txt bs=10M count=1

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

# Test 1: Basic transfer with good network conditions
echo -e "\n${BLUE}Test 1: Basic transfer with good network conditions${NC}"
simulate_network 10 100 0
run_transfer_test "test_files/small.txt" 1

# Test 2: Transfer with poor network conditions
echo -e "\n${BLUE}Test 2: Transfer with poor network conditions${NC}"
simulate_network 200 10 5
run_transfer_test "test_files/medium.txt" 2

# Test 3: Multiple transfers with different priorities
echo -e "\n${BLUE}Test 3: Multiple transfers with different priorities${NC}"
simulate_network 50 50 1

# Start multiple transfers with different priorities
run_transfer_test "test_files/small.txt" 3 &
run_transfer_test "test_files/medium.txt" 1 &
run_transfer_test "test_files/large.txt" 2 &

# Wait for all transfers to complete
wait

# Test 4: Transfer with network interruption
echo -e "\n${BLUE}Test 4: Transfer with network interruption${NC}"
simulate_network 10 100 0
run_transfer_test "test_files/large.txt" 1 &
local transfer_pid=$!

# Simulate network interruption after 5 seconds
sleep 5
simulate_network 1000 1 20
sleep 5
simulate_network 10 100 0

# Wait for transfer to complete
wait $transfer_pid

# Cleanup
echo -e "\n${BLUE}Cleaning up...${NC}"
cleanup_network
rm -rf test_files test_output

echo -e "${GREEN}Test suite completed${NC}" 