# Use rust image as base
FROM rust:latest

# Set working directory inside the container
WORKDIR /usr/src/app

# Copy the Rust project files into the container
COPY . .

# Build the Rust project
RUN cargo build --release --package sfu --example sync_chat

# Expose the TCP port the signal server will listen on
EXPOSE 8080
# Expose the UDP ports the media server will listen on
EXPOSE 3478-3495/udp

RUN mkdir -p logs

# Command to run the server
CMD ./target/release/examples/sync_chat -f -d --level info > ./logs/sfu.log 2>&1 & echo $! > server_pid.txt & cargo test --release --no-fail-fast -- --show-output > ./logs/test.log 2>&1
