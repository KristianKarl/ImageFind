FROM rust:latest
WORKDIR /bin/ImageFind

COPY src ./src
COPY templates ./templates
COPY Cargo.lock ./
COPY Cargo.toml ./

RUN cargo build --release

FROM ubuntu:latest
RUN apt update && apt install -y \
	sqlite3 \
	exiv2 \
	ffmpeg \
 && rm -rf /var/lib/apt/lists/*
COPY --from=0 /bin/ImageFind/target/release/ImageFind /bin/ImageFind
CMD ["ImageFind", "--scan-dir", "/scan-dir", "--db-path",  "/db/db.sqlite", "--thumbnail-cache", "/thumbnail-cache", "--full-image-cache", "/full-image-cache", "--video_preview-cache", "/video-preview-cache"]
