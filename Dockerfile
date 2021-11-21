# This dockerfile is designed for use in CI to build
# release artifacts. For testing, you should be able
# to get away with a normal `cargo build --release`.

# to match the version of Ubuntu used by the Actions runners.
FROM ubuntu:20.04

# This stuff almost never updates
ENV DISCORD_TOKEN="" GOOGLE_API_CREDENTIALS="" RUST_LOG=info DEBIAN_FRONTEND=noninteractive
LABEL org.opencontainers.image.source="https://github.com/SriRamanujam/discord-wavenet"

# This stuff doesn't update quite as often
COPY LICENSE /LICENSE
CMD ["/discord-wavenet"]

# This stuff updates a bunch
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y ca-certificates ffmpeg && \
    rm -rf /var/lib/apt/lists/*
COPY target/release/discord-wavenet /discord-wavenet
