# syntax=docker/dockerfile:1
#
# The Yardsort server's image: `yardsort-server` alone, on a distroless base, running as a user
# with no shell and no package manager. Built from the repository root, which holds the Cargo
# workspace:
#
#   docker build -f crates/server/Dockerfile -t yardsort-server .
#
# Configured by its environment (crates/server/src/config.rs); it listens on 8080. Staging deploys
# it to Fly.io (crates/server/fly/), and the self-host recipe builds it (crates/server/self-host/).
#
# Both bases are pinned by digest, so a build months from now is the same build. To move to newer
# ones, take the digests from `docker buildx imagetools inspect <image>:<tag>`, and keep the two on
# the same Debian release: the binary links against the build stage's glibc.

FROM rust:1.98.1-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS build
# The repository's rust-toolchain.toml asks for "stable", which rustup would download afresh; the
# image's own toolchain is the pinned one.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION}
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p yardsort-server --bin yardsort-server \
    && cp target/release/yardsort-server /yardsort-server

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
COPY --from=build /yardsort-server /usr/local/bin/yardsort-server
ENV YARDSORT_LISTEN=0.0.0.0:8080
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/yardsort-server"]
