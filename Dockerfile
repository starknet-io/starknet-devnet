FROM rust:1.92-alpine3.22 AS chef

RUN apk add --no-cache pkgconfig make musl-dev openssl-dev perl && \
    cargo install cargo-chef --locked

WORKDIR /app

FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --bin starknet-devnet --release

FROM alpine:3.22

# Use tini to avoid hanging process on Ctrl+C
# Use ca-certificates to allow forking from URLs using https scheme
RUN apk add --no-cache tini ca-certificates

RUN addgroup -S devnet && adduser -S devnet -G devnet

COPY --from=builder /app/target/release/starknet-devnet /usr/local/bin/starknet-devnet

USER devnet

# The default port; exposing is beneficial if using Docker GUI
EXPOSE 5050

ENTRYPOINT [ "tini", "--", "starknet-devnet", "--host", "0.0.0.0" ]
