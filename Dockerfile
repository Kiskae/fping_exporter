FROM rust:1.48 as planner
WORKDIR /app
RUN cargo install cargo-chef
RUN mkdir src && touch src/main.rs
COPY ./Cargo.toml ./Cargo.lock ./
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:1.48 as cacher
WORKDIR /app
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM rust:1.48 as builder
WORKDIR /app
COPY --from=cacher /app/target target
COPY --from=cacher $CARGO_HOME $CARGO_HOME
COPY . .
RUN cargo build --release --offline

# Compile the latest version of fping
FROM buildpack-deps:buster as fping_builder
WORKDIR /usr/src/fping
RUN curl https://fping.org/dist/fping-5.0.tar.gz | tar -xz --strip-components=1
RUN ./configure && make && make install

FROM debian:stable-slim
# netbase provides the required '/etc/protocols'
RUN apt-get update && apt-get install -y netbase && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fping_exporter /bin/
COPY --from=fping_builder /usr/local/sbin/fping /bin/
ENV FPING_BIN=/bin/fping
ENV RUST_BACKTRACE=1
ENTRYPOINT [ "/bin/fping_exporter" ]