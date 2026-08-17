# Contributing to OpenDroidMic

Thank you for your interest in contributing!

## Development Setup

### Linux
1. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. Install PipeWire dev packages for your distro
3. Clone and build: `cargo build`

### Android
1. Install Android Studio
2. Open the `android/` directory
3. Build and run

## Code Style

### Rust
- Follow standard `rustfmt` formatting
- Use `clippy` lints
- Add tests for new functionality

### Kotlin
- Follow Android/Kotlin conventions
- Use ViewBinding for UI
- Handle permissions at runtime

## Testing

```bash
# Linux unit tests
cd linux && cargo test

# Run test sender
cargo run --bin opendroidmic-test-sender
```

## Pull Requests

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Ensure all tests pass
6. Submit a pull request

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
