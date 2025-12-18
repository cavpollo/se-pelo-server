FROM rust:1.89.0

# Copy local code to the container image.
WORKDIR /usr/src/app
COPY ./src/ ./src/
COPY ./resources/ ./resources/
COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock

# Install production dependencies and build a release artifact.
RUN cargo install --path .

# Run the web service on container startup.
CMD ["se_pelo"]
