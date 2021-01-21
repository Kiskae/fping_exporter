ARG RUST_VERSION="1.49"
ARG FPING_VERSION="5.0"

# Download dependencies on native hardware, qemu and git don't play together
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION} as vendor
WORKDIR /app
RUN mkdir src && touch src/main.rs
COPY ./Cargo.toml ./Cargo.lock ./
RUN mkdir .cargo && cargo vendor > .cargo/config

# Compile a dummy application to cache dependencies
FROM rust:${RUST_VERSION} as cacher
WORKDIR /app
RUN mkdir src && echo "fn main() {}" > src/main.rs
COPY ./Cargo.toml ./Cargo.lock ./
COPY --from=vendor /app/.cargo .cargo
COPY --from=vendor /app/vendor vendor
RUN cargo build --release --offline --verbose

FROM rust:${RUST_VERSION} as builder
WORKDIR /app
COPY --from=cacher /app/target target
COPY --from=vendor /app/.cargo .cargo
COPY --from=vendor /app/vendor vendor
COPY . .
RUN cargo build --release --offline --verbose --features "docker"

# Compile the latest version of fping
FROM buildpack-deps:buster as fping_builder
WORKDIR /usr/src/fping
ARG FPING_VERSION
RUN curl https://fping.org/dist/fping-${FPING_VERSION}.tar.gz | tar -xz --strip-components=1
RUN ./configure && make && make install

FROM debian:stable-slim
# netbase provides the required '/etc/protocols'
RUN apt-get update && apt-get install -y netbase && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fping_exporter /bin/
COPY --from=fping_builder /usr/local/sbin/fping /bin/
ENV FPING_BIN=/bin/fping
ENV RUST_BACKTRACE=1
ENTRYPOINT [ "/bin/fping_exporter" ]