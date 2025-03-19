#!/bin/bash

# Hermes CLI wrapper script
# Makes it easier to use the Hermes secure messaging system

LISTEN_ADDR="0.0.0.0:9000"
CONFIG_FILE="$HOME/.hermes_config"
LAST_PEER=""

# Load previous peer if exists
if [ -f "$CONFIG_FILE" ]; then
    source "$CONFIG_FILE"
fi

show_help() {
    echo "Hermes Messaging - Simple Usage Guide"
    echo ""
    echo "Commands:"
    echo "  ./hermes.sh start             Start a node (listening on $LISTEN_ADDR)"
    echo "  ./hermes.sh connect <ID> <IP> Connect to a peer"
    echo "  ./hermes.sh send <message>    Send message to last connected peer"
    echo "  ./hermes.sh sendto <ID> <msg> Send message to specific peer"
    echo "  ./hermes.sh direct <ID> <IP> <msg> Connect and send in one step"
    echo "  ./hermes.sh port <PORT>       Change default port (currently ${LISTEN_ADDR#*:})"
    echo "  ./hermes.sh help              Show this help"
    echo ""
    echo "IMPORTANT: To receive messages, you must run 'start' command in a separate terminal."
    echo "           Messages will be displayed automatically in that terminal when received."
    echo ""
    echo "Examples:"
    echo "  ./hermes.sh start"
    echo "  ./hermes.sh connect abc123 192.168.1.100"
    echo "  ./hermes.sh send \"Hello world!\""
    echo "  ./hermes.sh direct abc123 192.168.1.100 \"Hello directly!\""
    echo ""
}

save_config() {
    echo "LISTEN_ADDR=\"$LISTEN_ADDR\"" > "$CONFIG_FILE"
    echo "LAST_PEER=\"$LAST_PEER\"" >> "$CONFIG_FILE"
}

if [ $# -lt 1 ]; then
    show_help
    exit 1
fi

case "$1" in
    start)
        echo "Starting Hermes node on $LISTEN_ADDR..."
        cargo run --bin hermes-cli -- start --listen "$LISTEN_ADDR"
        ;;
        
    connect)
        if [ $# -lt 3 ]; then
            echo "Error: Please provide peer ID and IP address"
            echo "Usage: ./hermes.sh connect <PEER_ID> <IP_ADDRESS>"
            exit 1
        fi
        PEER_ID="$2"
        IP_ADDR="$3"
        PORT="${4:-9000}"
        
        echo "Connecting to peer $PEER_ID at $IP_ADDR:$PORT..."
        cargo run --bin hermes-cli -- connect --peer "$PEER_ID" --address "$IP_ADDR:$PORT"
        
        # Save for later use
        LAST_PEER="$PEER_ID"
        save_config
        ;;
        
    send)
        if [ -z "$LAST_PEER" ]; then
            echo "Error: No peer to send to. Connect to a peer first."
            exit 1
        fi
        
        if [ $# -lt 2 ]; then
            echo "Error: Please provide a message to send"
            echo "Usage: ./hermes.sh send \"Your message here\""
            exit 1
        fi
        
        # Combine all arguments after "send" as the message
        shift
        MESSAGE="$*"
        
        echo "Sending message to $LAST_PEER..."
        cargo run --bin hermes-cli -- send --peer "$LAST_PEER" --message "$MESSAGE"
        ;;
        
    sendto)
        if [ $# -lt 3 ]; then
            echo "Error: Please provide peer ID and message"
            echo "Usage: ./hermes.sh sendto <PEER_ID> \"Your message here\" [IP_ADDRESS]"
            exit 1
        fi
        
        PEER_ID="$2"
        shift 2
        
        # Check if the last argument might be an IP address
        IP_ARG=""
        ARGS=("$@")
        LAST_ARG="${ARGS[${#ARGS[@]}-1]}"
        
        # Very basic check for IP-like format
        if [[ $LAST_ARG =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+ ]]; then
            IP_ARG="$LAST_ARG"
            # Remove the IP from the message arguments
            unset "ARGS[${#ARGS[@]}-1]"
            MESSAGE="${ARGS[*]}"
        else
            MESSAGE="$*"
        fi
        
        # If we have an IP, connect first
        if [ -n "$IP_ARG" ]; then
            echo "Connecting to peer $PEER_ID at $IP_ARG:9000..."
            cargo run --bin hermes-cli -- connect --peer "$PEER_ID" --address "$IP_ARG:9000"
        fi
        
        echo "Sending message to $PEER_ID..."
        cargo run --bin hermes-cli -- send --peer "$PEER_ID" --message "$MESSAGE"
        
        # Save for later use
        LAST_PEER="$PEER_ID"
        save_config
        ;;
        
    direct)
        if [ $# -lt 4 ]; then
            echo "Error: Please provide peer ID, IP address, and message"
            echo "Usage: ./hermes.sh direct <PEER_ID> <IP_ADDRESS> \"Your message here\""
            exit 1
        fi
        
        PEER_ID="$2"
        IP_ADDR="$3"
        shift 3
        MESSAGE="$*"
        
        echo "Connecting to peer $PEER_ID at $IP_ADDR:9000 and sending message..."
        # Use a single command to connect and send in one operation
        cargo run --bin hermes-cli -- direct --peer "$PEER_ID" --address "$IP_ADDR:9000" --message "$MESSAGE"
        
        # Save for later use
        LAST_PEER="$PEER_ID"
        save_config
        ;;
        
    port)
        if [ $# -lt 2 ]; then
            echo "Error: Please provide a port number"
            echo "Usage: ./hermes.sh port <PORT_NUMBER>"
            exit 1
        fi
        
        PORT="$2"
        LISTEN_ADDR="0.0.0.0:$PORT"
        echo "Default port set to $PORT"
        save_config
        ;;
        
    help|--help|-h)
        show_help
        ;;
        
    *)
        echo "Unknown command: $1"
        show_help
        exit 1
        ;;
esac

exit 0 