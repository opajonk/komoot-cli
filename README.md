# komoot-export

A Python script that downloads all tours from a [Komoot](https://www.komoot.com) account and saves them as GPX files, organised into subfolders by tour type and visibility.

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

- Python 3.11+

## Setup

```bash
uv sync --all-groups     # creates .venv and installs dependencies
source .venv/bin/activate
```

## Usage

Credentials can be passed as flags or via environment variables:

```bash
# flags
uv run export_tours.py --email you@example.com --password yourpassword

# environment variables
export KOMOOT_EMAIL=you@example.com
export KOMOOT_PASSWORD=yourpassword
uv run export_tours.py

# custom output directory (default: ./tours)
uv run export_tours.py --output-dir ~/komoot-backup
```

### Options

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--email` | `KOMOOT_EMAIL` | — | Komoot account e-mail (required) |
| `--password` | `KOMOOT_PASSWORD` | — | Komoot account password (required) |
| `--output-dir` | — | `tours` | Root directory for exported GPX files |

## Dependencies

- [kompy](https://github.com/Tsadoq/kompy) — Python wrapper for the Komoot API
- Development tooling is managed through `uv` dependency groups.
