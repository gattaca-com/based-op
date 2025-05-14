FROM debian:bookworm-slim AS runtime

# Install runtime dependencies and debugging tools
RUN apt-get update && apt-get install -y \
    ca-certificates \
    strace \
    gdb \
    vim \
    less

RUN mkdir /root/logs
RUN mkdir -p /app

WORKDIR /app

# Copy the built binary from the GitHub Actions workspace to the root directory
COPY ./output/overseer ./overseer
