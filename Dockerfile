FROM rust:latest

RUN apt-get update && apt-get install -y \
  cmake \
  clang \
  libssl-dev \
  zlib1g-dev \
  pkg-config \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

RUN cargo build

