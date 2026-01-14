FROM docker.io/library/rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler openssl libssl-dev

WORKDIR /work

COPY . .

RUN --mount=type=cache,target=/work/target \
    --mount=type=cache,target=/work/.cargo \
    cargo build --release --package tugboat-agent && cargo build --release --package tugboat-runtime && \
    cp /work/target/release/tugboat-agent /tugboat-agent && \
    cp /work/target/release/tugboat-runtime /tugboat-runtime

FROM debian:bookworm-slim AS cni-download

RUN apt-get update && apt-get install -y wget

WORKDIR /work

RUN wget https://github.com/containernetworking/plugins/releases/download/v1.9.0/cni-plugins-linux-amd64-v1.9.0.tgz && \
    echo '58c03705426e929658f45a851df15a86d06ef680cacbf3f2dc127731ca265c28  cni-plugins-linux-amd64-v1.9.0.tgz' > plugin.sum && \
    sha256sum --check plugin.sum && \
    mkdir /plugins && \
    tar xzvf cni-plugins-linux-amd64-v1.9.0.tgz -C /plugins

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y qemu-system qemu-utils && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /tugboat-agent /usr/bin/tugboat-agent
COPY --from=builder /tugboat-runtime /usr/bin/tugboat-runtime
COPY --from=cni-download /plugins/* /opt/cni/bin/*

CMD ["/usr/bin/tugboat-agent"]
