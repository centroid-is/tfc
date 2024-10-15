# framework-rs
Rust version of TFC framework


# Cross compile for aarch64

## Prerequisites

```
sudo pacman -S lld
```

## Add target

```
rustup target add aarch64-unknown-linux-musl
cargo build --target aarch64-unknown-linux-musl
```

Add this to `.cargo/config`

```
[target.aarch64-unknown-linux-musl]
linker = "lld"
```
