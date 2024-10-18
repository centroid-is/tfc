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


# Tokio Console

Add to Cargo.toml.
Tracing is optional, provides easy interface to instrument code.
```
tokio = { version = "1.39.2", features = ["rt-multi-thread", "macros", "tracing"] }
tracing = "0.1.40"
console-subscriber = "0.4.0"
```

Install tokio-console
```
cargo install tokio-console
```

Add this to your main function

```
console_subscriber::init();
```

Run the program, as of now, the config for tokio unstable is needed
please refer to https://tokio.rs/tokio/topics/tracing-next-steps for more information
```
RUSTFLAGS="--cfg tokio_unstable" cargo run
```

Open another terminal and run

```
~./cargo/bin/tokio-console
```

