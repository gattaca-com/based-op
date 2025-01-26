# Makes sure the nightly-2024-10-01 toolchain is installed
toolchain := "nightly-2024-10-01"

fmt:
  rustup toolchain install {{toolchain}} > /dev/null 2>&1 && \
  cargo +{{toolchain}} fmt

fmt-check:
  rustup toolchain install {{toolchain}} > /dev/null 2>&1 && \
  cargo +{{toolchain}} fmt --check

clippy:
  cargo clippy --all-features --no-deps -- -D warnings