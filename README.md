# ImageFind

ImageFind is a Rust (Actix Web) app that scans XMP sidecar files, indexes useful metadata in SQLite, and serves a searchable UI with on-demand thumbnails and previews for images and videos.

The project is created mainly by prompting/Github Copilot using Visual Code.

## Use case

As a user you want to be able to search and view your personal collection of images and videos.
* Your collection is managed by [Digikam](https://www.digikam.org/)
* [sidecars](https://docs.digikam.org/en/setup_application/metadata_settings.html#sidecars-settings) are enabled, so xmp sidecar files are created for every image.
* The collection is on disk on some local machine on your LAN.

This tool will enable you to quickly find and view any image or video in your collection.

## Features

- Scan a directory for .xmp sidecars and import metadata into SQLite
- Search via UI or JSON API with simple AND support
- On-demand thumbnail generation and cached full-size image previews
- Keyboard-friendly modal with navigation and rotation (images)
- Video preview and playback in modal using HTML5 `<video>` element
- Video preview uses pre-transcoded files from a dedicated cache directory
- Basic path security checks
- Configurable webserver port via CLI

## Getting Started

- Build:
  ```
  cargo build
  ```
- Run:
  ```
  cargo run -- --scan-dir /path/to/library --db-path /path/to/index.sqlite --thumbnail-cache /path/to/thumb_cache --full-image-cache /path/to/full_cache --video_preview-cache /path/to/video_preview_cache [--port 8080]
  ```
- The server listens on http://0.0.0.0:8080 by default (use `--port` to change).

### If you are on Nixos

- Build:
  ```
  devenv shell
  cargo build
  ```

### Usage

To be able to create thumbnails and previews from videos, ffmpeg needs to be installed (for manual transcoding).

```
imagefind --scan-dir <DIR> --db-path <FILE> --thumbnail-cache <DIR> --full-image-cache <DIR> --video_preview-cache <DIR> [--port <PORT>]
```

- All arguments except `--port` are required.
- ImageFind needs to run from the machine where the collection is on.
- The firewall on the machine needs to be opened for the chosen port.

### Pre-rendering of videos

There's a utility [bash script](utils/transcodePreviewVideos.sh) that should be run ahead of time to render smaller and normalized versions of the original videos.

Usage:

  `./transcodePreviewVideos.sh [scan_folder] [target_folder] [num_jobs]`

Arguments:
- scan_folder
  - Optional. Path to the folder to scan for video files. Defaults to current directory.
- target_folder
  - Optional. Path to the folder where transcoded videos will be saved. Defaults to scan folder. This is the same folder that `ImageFind` argument the `--video_preview-cache` should hold.
- num_jobs
  - Optional. Number of parallel transcoding jobs. Defaults to 5.

Output:
- Creates new files with _480p.mp4 suffix for each transcoded video in the target folder.
- Skips transcoding if the output file already exists in the target folder.

Example:
  `./transcodePreviewVideos.sh ~/Pictures/ ~/img_service/video_preview_cache/`

### CLI Arguments

- --scan-dir <DIR> (required)
  - Root directory to scan for .xmp sidecar files on startup.
  - Example: --scan-dir /mnt/photos
- --db-path <FILE> (required)
  - Path to the SQLite database used to store the index.
  - Example: --db-path /var/lib/imagefind/index.sqlite
- --thumbnail-cache <DIR> (required)
  - Directory to store generated thumbnails.
- --full-image-cache <DIR> (required)
  - Directory to store full-size image previews.
- --video_preview-cache <DIR> (required)
  - Directory to store pre-transcoded video previews (`_480p.mp4` files).
- --log-level <LEVEL> (optional)
  - Set the logging level (e.g., info, debug, trace). Defaults to `info`.
- --port <PORT> (optional)
  - Port for the webserver. Defaults to `8080`.

Optional (provided by clap)
- -h, --help
  - Print help information.
- -V, --version
  - Print version information.

## Database Schema

The application uses a simple SQLite database with two main tables to store indexed metadata.

- **`file` table**: Stores a record for each media file found.
  - `id` (INTEGER, PRIMARY KEY): A unique identifier for the file record.
  - `path` (TEXT, UNIQUE): The absolute path to the media file (e.g., `/path/to/image.jpg`).
  - `hash` (TEXT): An xxhash of the corresponding `.xmp` sidecar file's content. This is used to efficiently detect if the metadata has changed since the last scan.

- **`key_value` table**: Stores the extracted metadata tags as key-value pairs, linked to a file.
  - `id` (INTEGER, PRIMARY KEY): A unique identifier for the key-value pair.
  - `file_id` (INTEGER): A foreign key that references the `id` in the `file` table.
  - `key` (TEXT): The name of the metadata tag (e.g., `digiKam:TagsList`).
  - `value` (TEXT): The value of the metadata tag (e.g., `vacation`).

This schema allows for flexible querying of metadata across all indexed files.

## How it works

The application's workflow is divided into two main phases: indexing and serving.

### 1. Indexing on Startup

When the application starts, it performs a scan of the directory specified by `--scan-dir`.

- **File Discovery**: It recursively searches for `.xmp` sidecar files. For each `.xmp` file found, it determines the path to the corresponding media file (e.g., `image.jpg.xmp` -> `image.jpg`).
- **Change Detection**: It calculates an xxhash of the `.xmp` file's content. This hash is compared against the stored hash in the `file` table for that media path. If the hash is unchanged, the file is skipped, making subsequent scans much faster.
- **Metadata Extraction**: If the file is new or has changed, it parses the `.xmp` file to extract key metadata fields, such as:
  - `xmp:ModifyDate`
  - `digiKam:TagsList` (each tag is stored as a separate key-value pair)
  - `dc:title`
- **Database Update**: The extracted metadata is stored in the `key_value` table, associated with the file's ID from the `file` table.

### 2. Serving Content and Search

Once indexing is complete, the Actix Web server starts and listens for requests.

- **Search**: The UI (`/search`) and API (`/api`) endpoints accept a `search` query parameter. The query string is split by ` AND ` to support multi-term searches. The application then queries the `key_value` table for files that have metadata values matching all provided terms.
- **Thumbnail Generation**: The search results page loads asynchronously, with each result item making a request to `/thumbnail/{path}`. The server checks a local cache (`thumbnail_cache/`) for an existing thumbnail. If not found, it generates a new thumbnail from the media file, saves it to the cache, and returns it as a Base64-encoded string in a JSON response.
- **Image and Video Previews**: Clicking a result in the UI opens a modal preview.
  - For images, a request is made to `/image/{path}`. The server generates and caches a full-size JPEG preview in `full_image_cache/`, serving it with an `image/jpeg` content type.
  - For videos, a request to `/video/{path}` serves a pre-transcoded video file (`_480p.mp4`) from the `video_preview_cache` directory for browser playback. The browser's native `<video>` player is used for playback in the modal.
- **Caching**: Both thumbnail and full-image preview generation are computationally intensive. The disk-based caches at `--thumbnail-cache`, `--full-image-cache`, and `--video_preview-cache` significantly improve performance on subsequent requests for the same media. A cache-busting parameter (`?t=timestamp`) can be added to image URLs to force regeneration.

## Video Preview Logic

- When a video is requested for preview, the backend looks for a file with `_480p.mp4` appended to the basename (e.g., `video.mp4` → `video_480p.mp4`) in the `video_preview_cache` directory.
- If the `_480p.mp4` file exists, it is served as the video preview.
- If not, a 404 is returned and no transcoding is performed automatically.
- You must manually transcode videos to this format and place them in the cache directory.

Example ffmpeg command to create a compatible preview file:
```
ffmpeg -i input.mp4 -vf "scale=-2:480" -c:v libx264 -preset fast -profile:v main -pix_fmt yuv420p -b:v 1M -r 25 output_480p.mp4
```
Then move `output_480p.mp4` to your `video_preview_cache` directory.

## Endpoints

- GET /
  - Index page (redirects to /search when search is present).
- GET /search?search=term
  - HTML results grid with async thumbnails and modal.
- GET /api?search=term
  - JSON: [{ file_path, value, thumbnail_base64 }]
- GET /thumbnail/{path}
  - JSON: { thumbnail: base64 or null, file_path }
- GET /image/{path}
  - image/jpeg preview (cached). Supports cache-busting param t.
- GET /video/{path}
  - Serves a pre-transcoded video preview (`_480p.mp4` file from cache).
- GET /health_check
  - Returns “Healthy”.

### Request-time parameters

- Search query
  - /search?search=term
  - AND must be uppercase and separated by spaces (e.g., foo AND bar).
- Cache busting
  - /image/{path}?t=timestamp forces regeneration/refresh.

## Notes

- Media-serving routes apply basic path traversal prevention.
- Ensure the process can read the media files you reference.
- Video previews require manual transcoding to `_480p.mp4` files and placement in the cache directory.

- Closing the modal window stops video playback and audio.

## Troubleshooting

- Thumbnails don’t load
  - Check server logs and file permissions.
- Previews fail
  - Inspect /image or /video requests in dev tools Network tab.
  - Force refresh: /image/{path}?t=1690000000000
- No results
  - Confirm --scan-dir contains .xmp files and DB path is writable.
- Video preview not working
  - Ensure the `_480p.mp4` file exists in the `video_preview_cache` directory.
  - Check that the requested path is absolute and matches the file on disk.

## Todo

- More robust video format support and fallback.
- Improved error handling for video preview.
- Optionally add auto-transcoding as a background task.

## License

MIT
