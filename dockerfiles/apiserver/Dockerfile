FROM docker.io/library/rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler openssl libssl-dev

WORKDIR /work

COPY . .

RUN --mount=type=cache,target=/work/target \
    --mount=type=cache,target=/work/.cargo \
    cargo build --release --package tugboat-apiserver && \
    cp /work/target/release/tugboat-apiserver /tugboat-apiserver

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y openssl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /tugboat-apiserver /usr/bin/tugboat-apiserver

CMD ["/usr/bin/tugboat-apiserver"]
