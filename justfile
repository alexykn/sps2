set positional-arguments

help:
    just -l

fetch:
    rustup show active-toolchain
    cargo fetch

fmt *args:
    rustup show active-toolchain
    cargo fmt "$@"

check *args:
    rustup show active-toolchain
    cargo check

lint *args:
    rustup show active-toolchain
    cargo clippy --all-targets --all-features "$@"

fix *args:
    rustup show active-toolchain
    cargo clippy --fix --all-targets --all-features --allow-dirty "$@"

build *args:
    rustup show active-toolchain
    cargo build --release --target=aarch64-apple-darwin

test *args:
    rustup show active-toolchain
    cargo test

push:
    git push origin main

pull:
    git pull origin main
