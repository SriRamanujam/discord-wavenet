FROM rust:1.51 AS builder
WORKDIR /code
COPY . .
RUN cargo build --release

FROM debian:buster-slim
ENV DISCORD_TOKEN="" GOOGLE_API_CREDENTIALS="" RUST_LOG=info DEBIAN_FRONTEND=noninteractive
LABEL org.opencontainers.image.source="https://github.com/SriRamanujam/discord-wavenet"
RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y ca-certificates ffmpeg && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /code/LICENSE /LICENSE
COPY --from=builder /code/target/release/discord-wavenet /discord-wavenet
CMD ["/discord-wavenet"]
