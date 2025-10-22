# Contributing to btck-rust-node

Thank you for your interest in contributing to btck-rust-node!

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/yourusername/btck-rust-node.git`
3. Create a branch: `git checkout -b feature/your-feature`
4. Make your changes
5. Run tests: `cargo test`
6. Format code: `cargo fmt`
7. Run clippy: `cargo clippy`
8. Commit: `git commit -m "Description of changes"`
9. Push: `git push origin feature/your-feature`
10. Create a Pull Request

## Code Style

- Follow Rust standard style guidelines
- Use `cargo fmt` before committing
- Ensure `cargo clippy` passes without warnings
- Write tests for new functionality
- Document public APIs

## Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with logging
RUST_LOG=debug cargo test -- --nocapture
```

## Pull Request Process

1. Ensure all tests pass
2. Update documentation if needed
3. Describe your changes in the PR description
4. Link any related issues
5. Wait for review

## Development Priorities

### High Priority
- Complete P2P message handling
- Mempool implementation
- Transaction relay

### Medium Priority
- Wallet functionality
- Mining support
- Additional RPC methods

### Low Priority
- Performance optimization
- GUI
- Advanced features

## Questions?

Feel free to open an issue for discussion!
