# syntax=docker/dockerfile:1
#
# The headless host's image: `yardsort-host` and `ys` on Debian slim, with `git`, running as an
# ordinary user under `tini`. Built from the repository root, which holds the Cargo workspace:
#
#   docker build -f crates/host/Dockerfile -t yardsort-host .
#   docker build -f crates/host/Dockerfile --build-arg UID="$(id -u)" --build-arg GID="$(id -g)" -t yardsort-host .
#
# The second makes its user the repositories' owner, so what the agents write is yours.
#
# No agent is in it, and none of their sign-ins: build `FROM` it to add the ones you use, and mount
# their sign-ins, or start from the full image (Dockerfile.full), which has the tools agents use and
# the harnesses that may be redistributed. Each release publishes both, as
# ghcr.io/joaoh82/yardsort-host and ghcr.io/joaoh82/yardsort-host-full, with UID and GID 1000. It publishes no port, since the host only reaches out. Never give it the Docker
# socket. Its volumes, each its own: /data (the profile and settings.toml), /credentials (the host's
# sign-in), /worktrees, and the repositories wherever you mount them. Stopping the container ends
# the agents too: the terminal daemon lives inside it. See docs/guide/headless-host.md, and
# compose.yaml beside this file.
#
# Both bases are pinned by digest, and kept on the same Debian release: the binaries link against
# the build stage's glibc. The build stage is the server's (crates/server/Dockerfile); move the two
# together. Digests come from `docker buildx imagetools inspect <image>:<tag>`.

FROM rust:1.98.1-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS build
# The repository's rust-toolchain.toml asks for "stable", which rustup would download afresh; the
# image's own toolchain is the pinned one.
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION}
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked -p yardsort-host -p yardsort-cli \
    && cp target/release/yardsort-host target/release/ys /usr/local/bin/

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
# git for the worktrees, ssh to push and fetch, certificates to reach the server, tini to reap what
# the daemon's programs leave behind.
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       ca-certificates git openssh-client tini \
    && rm -rf /var/lib/apt/lists/*

ARG UID=1000
ARG GID=1000
RUN groupadd --gid "$GID" yardsort \
    && useradd --uid "$UID" --gid "$GID" --create-home --shell /bin/bash yardsort \
    && install -d -o "$UID" -g "$GID" -m 700 /data /credentials \
    && install -d -o "$UID" -g "$GID" -m 755 /worktrees

COPY --from=build /usr/local/bin/yardsort-host /usr/local/bin/ys /usr/local/bin/

# The source label is also what links the published package to the repository.
LABEL org.opencontainers.image.title="yardsort-host" \
      org.opencontainers.image.description="Yardsort's headless host: a computer with no screen that your phone reaches" \
      org.opencontainers.image.source="https://github.com/joaoh82/yardsort" \
      org.opencontainers.image.documentation="https://yardsort.sh/docs/guide/headless-host" \
      org.opencontainers.image.licenses="GPL-3.0-only"

ENV YARDSORT_DATA_DIR=/data \
    YARDSORT_CREDENTIALS_DIR=/credentials \
    YARDSORT_WORKTREE_ROOT=/worktrees \
    SHELL=/bin/bash
USER yardsort
WORKDIR /home/yardsort
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/yardsort-host"]
CMD ["serve"]
