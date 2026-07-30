# Third-party software

The Linux Hub build statically links the following software into the
`libertas-smart_building_hvac` application artifact.

## XGBoost 3.0.0

- Project: <https://github.com/dmlc/xgboost>
- License: Apache License 2.0
- Source and license:
  [`vendor/xgboost_lib-sys/xgboost`](vendor/xgboost_lib-sys/xgboost)
- Build: CPU-only static library, position-independent code, CUDA disabled,
  OpenMP disabled

## rust-xgboost wrapper and system binding

- Project: <https://github.com/marcomq/rust-xgboost>
- Safe wrapper version: 3.0.5
- System binding version: 3.0.4
- License: MIT
- Local change: the build adapter compiles the bundled XGBoost source as a
  static library instead of downloading or linking a shared library.
- License:
  [`vendor/xgboost_lib-sys/LICENSE-MIT`](vendor/xgboost_lib-sys/LICENSE-MIT)

The application uses the published safe `xgb` wrapper. Unsafe C API calls
remain in that dependency and do not appear in application source.

## Ubuntu 26.04 build host

The Hub-equivalent ARM64 Incus build uses these Ubuntu packages:

- `rustc`
- `cargo`
- `cmake`
- `g++`
- `libclang-dev`
- `pkg-config`

`rustfmt` and `rust-clippy` are also installed for source verification. These
are build-host dependencies, not application runtime dependencies.

The verified ARM64 ELF embeds the XGBoost C API and has no shared XGBoost,
OpenMP, or CUDA dependency. Its remaining shared libraries are the Ubuntu base
system's `libstdc++`, `libgcc_s`, `libm`, `libc`, and ELF loader.
