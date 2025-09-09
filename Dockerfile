FROM rust:latest
WORKDIR /bin/ImageFind

COPY src ./src
COPY templates ./templates
COPY Cargo.lock ./
COPY Cargo.toml ./

RUN cargo build --release

FROM ubuntu:latest
RUN apt update && apt install sqlite3 -y
COPY --from=0 /bin/ImageFind/target/release/ImageFind /bin/ImageFind
CMD ["ImageFind", "--scan-dir", "/scan-dir", "--db-path",  "/db/db.sqlite", "--thumbnail-cache", "/thumbnail-cache", "--full-image-cache", "/full-image-cache", "--video-preview-cache", "/video-preview-cache"]
