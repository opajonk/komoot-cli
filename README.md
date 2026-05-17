# komoot-export

A Rust CLI that downloads all tours from a [Komoot](https://www.komoot.com) account and saves them as GPX files, organised into subfolders by tour type and visibility.

## Output structure

```
tours/
├── planned/
│   ├── public/
│   ├── friends/
│   └── private/
└── made/
    ├── public/
    ├── friends/
    └── private/
```

Each file is named `YYYY-MM-DD_{tourId}_{tourName}.gpx`.
Re-running the script is safe — already-downloaded files are skipped.

## Requirements

- Rust (stable toolchain)

## Setup

```bash
cargo build
```

## Usage

Credentials can be passed as flags or via environment variables:

```bash
# flags
cargo run -- --email you@example.com --password yourpassword

# environment variables
export KOMOOT_EMAIL=you@example.com
export KOMOOT_PASSWORD=yourpassword
cargo run --

# custom output directory (default: ./tours)
cargo run -- --output-dir ~/komoot-backup
```

### Options

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--email` | `KOMOOT_EMAIL` | — | Komoot account e-mail (required) |
| `--password` | `KOMOOT_PASSWORD` | — | Komoot account password (required) |
| `--output-dir` | — | `tours` | Root directory for exported GPX files |

## Development

- Run formatting check:
  - `cargo fmt --all -- --check`
- Run lints:
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Run tests:
  - `cargo test --all-targets --all-features`

## Notes

- This implementation talks directly to Komoot HTTP endpoints from Rust (no Python/runtime interop).
