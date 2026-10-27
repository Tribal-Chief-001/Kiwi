# Contributing to Kiwi 🥝

Thank you for your interest in contributing to **Kiwi**, the zero-footprint CLI network intelligence and speed test engine!

---

## 🛠️ Development Setup

### Prerequisites
- **Rust Toolchain**: `rustc` and `cargo` (Rust 2024 edition or 1.85+).
- **Git**

### Build from Source
```bash
# Clone the repository
git clone https://github.com/Tribal-Chief-001/Kiwi.git
cd Kiwi

# Run debug build
cargo build

# Run release build with full LTO and stripped symbols
cargo build --release
```

---

## 🧪 Testing & Verification

Before submitting pull requests, ensure all tests, formatting, and linter checks pass:

```bash
# Run all unit and integration tests (15 tests)
cargo test

# Check code formatting
cargo fmt --check

# Check linter warnings with strict denials
cargo clippy --all-targets -- -D warnings
```

---

## 📐 Architecture Principles

When contributing code, adhere to Kiwi's core engineering tenets:
1. **Zero Terminal Hijacking**: Never enter alternate screen buffers (`smcup`/`rmcup`). Rendering must occur in-place via relative ANSI cursor overwrites.
2. **Sub-4MB RSS Footprint**: Avoid heap allocations in hot loops. Reuse stack ring buffers (`[u8; 16384]`) and static upload slices.
3. **Cooperative Scheduling**: Always check atomic yield flags (`PROBE_PENDING`) during saturation loops to prevent bulk traffic from starving latency probes.
4. **Cross-Platform Parity**: Code must compile and run seamlessly on Linux (glibc & musl), macOS, and Windows.

---

## 📝 Commit Conventions

We follow Conventional Commits formatted as comprehensive project ledgers:
- Format: `<type>(<scope>): <concise summary>`
- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `ci`

---

## 📄 License

By contributing to Kiwi, you agree that your contributions will be licensed under the [MIT License](LICENSE).
