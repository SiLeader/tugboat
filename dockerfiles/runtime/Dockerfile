FROM docker.io/library/rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler openssl libssl-dev

WORKDIR /work

COPY . .

RUN --mount=type=cache,target=/work/target \
    --mount=type=cache,target=/work/.cargo \
    cargo build --release --package tugboat-runtime && \
    cp /work/target/release/tugboat-runtime /tugboat-runtime

FROM debian:bookworm-slim

COPY --from=builder /tugboat-runtime /usr/bin/tugboat-runtime

CMD ["/usr/bin/tugboat-runtime"]
