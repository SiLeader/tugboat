# Copyright 2025- SiLeader (Cerussite).
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

FROM docker.io/library/rust:1.92-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler openssl libssl-dev

WORKDIR /work

COPY . .

RUN --mount=type=cache,target=/work/target \
    --mount=type=cache,target=/work/.cargo \
    cargo build --release --package tugboat-scheduler && \
    cp /work/target/release/tugboat-scheduler /tugboat-scheduler

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y openssl curl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /tugboat-scheduler /usr/bin/tugboat-scheduler

CMD ["/usr/bin/tugboat-scheduler"]
