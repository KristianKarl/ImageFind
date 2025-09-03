#!/bin/bash
# transcodePreviewVideos.sh
# ---------------------------------------------
# Transcodes video files in a specified folder to 480p using ffmpeg and CUDA acceleration.
# Allows parallel transcoding with configurable number of jobs.
#
# Usage:
#   ./transcodePreviewVideos.sh [scan_folder] [target_folder] [num_jobs]
#
# Arguments:
#   scan_folder    Optional. Path to the folder to scan for video files. Defaults to current directory.
#   target_folder  Optional. Path to the folder where transcoded videos will be saved. Defaults to scan folder.
#   num_jobs       Optional. Number of parallel transcoding jobs. Defaults to 5.
#
# Output:
#   Creates new files with _480p.mp4 suffix for each transcoded video in the target folder.
#   Skips transcoding if the output file already exists in the target folder.
#
# Requirements:
#   ffmpeg with CUDA support must be installed.
# ---------------------------------------------


command -v ffmpeg >/dev/null 2>&1 || { echo >&2 "I require ffmpeg but it's not installed. Aborting."; exit 1; }

SAVEIFS=$IFS
IFS=$(echo -en "\n\b")

task() {
    local file="$1"
    local base_name
    base_name=$(basename "${file%.*}")
    local out_file="$TARGET_DIR/${base_name}_480p.mp4"
    if [ -f "$out_file" ]; then
      echo "Skipping $file, output $out_file already exists."
    else
      echo "Transcoding $file to 480p -> $out_file"
      ffmpeg -hide_banner -loglevel error -hwaccel cuda -hwaccel_output_format cuda -i "$file" -vf scale_cuda=-2:480 -c:v hevc_nvenc -preset p6 -r 25 -c:a aac -b:a 128k "$out_file"

      # No hardware acceleration
      #ffmpeg -hide_banner -loglevel error -i "$file" -vf scale=-2:480 -c:v libx264 -preset medium -r 25 -c:a aac -b:a 128k "$out_file"
    fi
}

# Get folder path from first argument, default to current directory
SCAN_DIR="${1:-.}"
# Get target folder from second argument, default to SCAN_DIR
TARGET_DIR="${2:-$SCAN_DIR}"
# Get number of jobs from third argument, default to 5
MAX_JOBS="${3:-5}"

# Create target directory if it doesn't exist
mkdir -p "$TARGET_DIR"

# Find all video files in the specified folder

job_count=0
for file in $( find "$SCAN_DIR" -type f -iregex '^.*\.AVI\|^.*\.MP4\|^.*\.MOV\|^.*\.3GP|^.*\.MKV' | sort )
do
  task "$file"&
  job_count=$((job_count+1))
  if [ "$job_count" -ge "$MAX_JOBS" ]; then
    wait -n
    job_count=$((job_count-1))
  fi
done
wait

IFS=$SAVEIFS

exit 0
