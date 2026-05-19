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

- Rust (stable toolchain), if you want to build or run via Cargo

## Setup

```bash
cargo build
```

Alternatively, you can download a prebuilt executable for your platform from the latest GitHub release:

<https://github.com/opajonk/komoot-export/releases/latest>

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
| `--from-date` | — | — | Only export tours on or after this date (`YYYY-MM-DD`) |
| `--to-date` | — | — | Only export tours on or before this date (`YYYY-MM-DD`) |
| `--status` | — | all | Only export tours with the given visibility; comma-separated (`public,friends,private`) |
| `--type` | — | all | Only export tours of the given type; comma-separated (`planned,recorded`) |

All filter flags are optional and can be combined freely. Tours excluded by a filter are never downloaded.

```bash
# export only public recorded tours from 2024
cargo run -- --email you@example.com --password yourpassword \
  --type recorded --status public \
  --from-date 2024-01-01 --to-date 2024-12-31
```

## Development

- Run formatting check:
  - `cargo fmt --all -- --check`
- Run lints:
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Run tests:
  - `cargo test --all-targets --all-features`
