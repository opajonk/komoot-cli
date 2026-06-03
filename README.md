# komoot-cli

A Rust CLI for interacting with [Komoot](https://www.komoot.com).

## Commands

### `routes export`

Downloads all tours from a Komoot account and saves them as GPX files, organised into subfolders by tour type and visibility.

### `routes list`

Lists all tours as a Markdown table to stdout.

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
Re-running the command is safe — already-downloaded files are skipped.

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
cargo run -- --email you@example.com --password yourpassword routes export

# environment variables
export KOMOOT_EMAIL=you@example.com
export KOMOOT_PASSWORD=yourpassword
cargo run -- routes export

# custom output directory (default: ./tours)
cargo run -- --email you@example.com routes export --output-dir ~/komoot-backup

# list all tours as markdown
cargo run -- --email you@example.com --password yourpassword routes list
```

### Global options for `komoot-cli`

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--email` | `KOMOOT_EMAIL` | — | Komoot account e-mail (required) |
| `--password` | `KOMOOT_PASSWORD` | — | Komoot account password (optional; prompted if missing) |

### Options for `routes export`

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--output-dir` | — | `tours` | Root directory for exported GPX files |
| `--from-date` | — | — | Only export tours on or after this date (`YYYY-MM-DD`) |
| `--to-date` | — | — | Only export tours on or before this date (`YYYY-MM-DD`) |
| `--status` | — | all | Only export tours with the given visibility; comma-separated (`public,friends,private`) |
| `--type` | — | all | Only export tours of the given type; comma-separated (`planned,recorded`) |

All filter flags are optional and can be combined freely. Tours excluded by a filter are never downloaded.

```bash
# export only public recorded tours from 2024
cargo run -- --email you@example.com --password yourpassword routes export \
  --type recorded --status public \
  --from-date 2024-01-01 --to-date 2024-12-31
```

### Options for `routes list`

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--from-date` | — | — | Only list tours on or after this date (`YYYY-MM-DD`) |
| `--to-date` | — | — | Only list tours on or before this date (`YYYY-MM-DD`) |
| `--status` | — | all | Only list tours with the given visibility; comma-separated (`public,friends,private`) |
| `--type` | — | all | Only list tours of the given type; comma-separated (`planned,recorded`) |

All filter flags are optional and can be combined freely.

## Development

- Run formatting check:
  - `cargo fmt --all -- --check`
- Run lints:
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Run tests:
  - `cargo test --all-targets --all-features`
