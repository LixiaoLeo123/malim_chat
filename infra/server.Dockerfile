FROM rust:1.85-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY apps/server/Cargo.toml apps/server/Cargo.toml
RUN mkdir -p apps/server/src && printf 'fn main() {}' > apps/server/src/main.rs && cargo build --release -p malim-server
COPY apps/server apps/server
RUN cargo build --release -p malim-server

FROM debian:bookworm-slim
RUN useradd --system --create-home --uid 10001 malim
COPY --from=build /app/target/release/malim-server /usr/local/bin/malim-server
USER malim
EXPOSE 3100
ENTRYPOINT ["/usr/local/bin/malim-server"]
