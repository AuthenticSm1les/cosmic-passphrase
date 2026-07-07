default: build

build:
    cargo build --release
    @echo "Done: pinentry-cosmic and cosmic-ssh-askpass"

build-all:
    cargo build

check:
    cargo check

test:
    cargo test

test-pinentry:
    cargo test -p pinentry-cosmic

clean:
    cargo clean

fmt:
    cargo fmt

lint:
    cargo clippy -- -D warnings

install-pinentry: build
    sudo install -m 755 target/release/pinentry-cosmic /usr/local/bin/pinentry-cosmic
    @echo "Installed pinentry-cosmic to /usr/local/bin/pinentry-cosmic"

install-ssh: build
    sudo install -Dm 755 target/release/cosmic-ssh-askpass /usr/lib/cosmic-ssh-askpass
    @echo "Installed cosmic-ssh-askpass to /usr/lib/cosmic-ssh-askpass"

install-all: install-pinentry install-ssh
