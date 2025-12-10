default:
    @just --list

alias b  := build
alias br := build-release
alias r  := run
alias rr := run-release
alias c  := check
alias t  := test

build:
    cargo build

build-release:
    cargo build --release

run *args:
    cargo run -- {{args}}

run-release *args:
    cargo run --release -- {{args}}

check:
    cargo check

test:
    cargo test

clean:
    cargo clean

fmt:
    cargo fmt

lint:
    cargo clippy -- -W clippy::pedantic

install:
    cargo install --path . --force
    @echo "Inro installed system-wide!"

uninstall:
    cargo uninstall inro
    @echo "Inro uninstalled."
