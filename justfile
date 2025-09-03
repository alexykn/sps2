set positional-arguments

help:
    just -l

fetch:
    rustup show active-toolchain
    cargo fetch

lint *args:
    cargo clippy --all-targets --all-features "$@"

fix *args:
    cargo clippy --fix --all-targets --all-features --allow-dirty "$@"

build *args:
    cargo build --release --target=aarch64-apple-darwin
