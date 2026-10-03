FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/* && useradd -m radar
COPY --from=build /app/target/release/rwa-radar-api /usr/local/bin/
USER radar
EXPOSE 8080
CMD ["rwa-radar-api"]
