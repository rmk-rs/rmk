#!/bin/bash

# Exit on any error
set -e

# Build and clean examples (except ESP32S3)
for dir in examples/use_rust/*/ examples/use_config/*/; do
    if [ -d "$dir" ] && [ -d "$dir/src" ]; then
        # Skip ESP32S3 (Xtensa) projects for now
        if [[ "$dir" == *"sf32"* ]]; then
            continue
        fi
        if [[ "$dir" == *"esp32s3_"* ]]; then
            continue
        fi
        cd "$dir"
        cargo build --release
        cd ../../..
    fi
done

# Setup ESP environment and build ESP32S3 projects
. ~/export-esp.sh

# Build ESP32S3 projects
for dir in examples/use_rust/*/ examples/use_config/*/; do
    if [ -d "$dir" ] && [ -d "$dir/src" ]; then
        if [[ "$dir" == *"esp32s3_"* ]]; then
            cd "$dir"
            cargo +esp build --release
            cd ../../..
        fi
    fi
done
