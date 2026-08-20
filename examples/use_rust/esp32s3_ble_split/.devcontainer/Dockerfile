FROM espressif/idf-rust:esp32s3_latest

# Avoid prompts from apt during build
ENV DEBIAN_FRONTEND=noninteractive

# Install additional system-level packages
RUN cargo install --force cargo-make

# [Optional] Set the default user if provided by the base image
USER esp
