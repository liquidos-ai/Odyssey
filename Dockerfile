FROM rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bubblewrap \
        build-essential \
        ca-certificates \
        git \
        libssl-dev \
        pkg-config \
        ripgrep \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash odyssey \
    && chown -R odyssey:odyssey /usr/local/cargo /usr/local/rustup

USER odyssey
WORKDIR /workspace

ENV HOME=/home/odyssey

CMD ["bash"]
