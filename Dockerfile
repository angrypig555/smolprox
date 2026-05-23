FROM rust:slim-trixie AS builder

WORKDIR /usr/src/smolprox
COPY . .
RUN cargo install --path .

FROM debian:trixie-slim
COPY --from=builder /usr/local/cargo/bin/smolprox /usr/local/bin/smolprox
EXPOSE 8080
CMD ["smolprox", "--nolog"]