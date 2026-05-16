#!/usr/bin/env python3
"""
Download all tours from a Komoot account and save them as GPX files,
sorted into subfolders by type and visibility:
  - planned/public/
  - planned/friends/
  - planned/private/
  - made/public/
  - made/friends/
  - made/private/
"""

import argparse
import getpass
import os
import re
import sys
import logging
from pathlib import Path

import types
from datetime import datetime, timezone

from kompy import KomootConnector
from kompy.constants.query_parameters import TourQueryParameters
from kompy.constants.tour_constants import TourTypes
from kompy.constants.privacy_status import PrivacyStatus
from kompy.constants.tour_object_types import TourObjectTypes


class _SuppressFilter(logging.Filter):
    """Drop specific noisy log messages from third-party libraries."""

    _SUPPRESSED = frozenset(
        [
            "No vector map image found.",
            "No sort field provided, using default sort field: date",
        ]
    )

    def filter(self, record: logging.LogRecord) -> bool:
        return record.getMessage() not in self._SUPPRESSED


logging.root.setLevel(logging.INFO)
# Remove any handlers kompy or other imports may have added to root, then
# install our own so format and filtering are always applied.
logging.root.handlers.clear()
_handler = logging.StreamHandler()
_handler.setFormatter(logging.Formatter("%(asctime)s [%(levelname)s] %(message)s"))
_handler.addFilter(_SuppressFilter())
logging.root.addHandler(_handler)
logger = logging.getLogger(__name__)

SUBFOLDER_MAP = {
    TourTypes.TOUR_PLANNED: "planned",
    TourTypes.TOUR_RECORDED: "made",
}

ALL_STATUSES = [
    PrivacyStatus.PUBLIC,
    PrivacyStatus.FRIENDS,
    PrivacyStatus.PRIVATE,
]


def sanitize_filename(name: str) -> str:
    """Replace characters that are unsafe in filenames with underscores."""
    return re.sub(r"[^\w\-. ]", "_", name).strip().replace(" ", "_")


def _parse_date(date_str: str) -> datetime:
    """Parse an ISO-8601 date string; returns a fallback date on any error."""
    try:
        return datetime.fromisoformat(date_str.replace("Z", "+00:00"))
    except (ValueError, AttributeError):
        return datetime(2000, 1, 1, tzinfo=timezone.utc)


def fetch_all_tours(connector: KomootConnector, user_identifier: str) -> list[tuple]:
    """
    Fetch all tours by directly paging the raw Komoot API, bypassing kompy's
    Tour object model (which applies strict validation that rejects some API
    responses, in particular for tour_planned entries).
    Returns a deduplicated list of (SimpleNamespace, actual_status) tuples.
    """
    seen_ids: set = set()
    all_tours = []

    for status in ALL_STATUSES:
        for tour_type in [TourTypes.TOUR_PLANNED, TourTypes.TOUR_RECORDED]:
            page = 0
            while True:
                try:
                    params = {
                        TourQueryParameters.TYPE: tour_type,
                        TourQueryParameters.STATUS: status,
                        TourQueryParameters.PAGE: page,
                    }
                    raw = connector._get_page_of_tours(
                        query_parameters=params,
                        user_identifier=user_identifier,
                    ).json()

                    raw_tours = raw.get("_embedded", {}).get("tours", [])
                    page_info = raw.get("page", {})
                    total_pages = page_info.get("totalPages", 1)
                    current_page_num = page_info.get("number", 0)

                    for td in raw_tours:
                        tour_id = str(td.get("id", ""))
                        if not tour_id or tour_id in seen_ids:
                            continue
                        seen_ids.add(tour_id)
                        # NOTE: _get_page_of_tours is a private kompy API;
                        # pin the kompy version and re-test on upgrades.
                        _api_status = td.get("status")
                        actual_status = (
                            _api_status if _api_status is not None else status
                        )
                        tour = types.SimpleNamespace(
                            id=tour_id,
                            name=td.get("name", "Unknown"),
                            type=td.get("type", tour_type),
                            start_date=_parse_date(td.get("date", "")),
                        )
                        all_tours.append((tour, actual_status))

                    logger.info(
                        "Fetched %d tours (type=%s, status=%s, page=%d/%d).",
                        len(raw_tours),
                        tour_type,
                        status,
                        current_page_num + 1,
                        max(total_pages, 1),
                    )

                    if current_page_num + 1 >= total_pages:
                        break
                    page = current_page_num + 1

                except Exception as exc:  # noqa: BLE001
                    logger.warning(
                        "Could not fetch tours (type=%s, status=%s, page=%d): %s",
                        tour_type,
                        status,
                        page,
                        exc,
                    )
                    break

    return all_tours


def download_tour_gpx(connector: KomootConnector, tour_id: str) -> bytes | None:
    """Download a single tour as raw GPX bytes."""
    try:
        gpx = connector.get_tour_by_id(
            tour_identifier=tour_id,
            object_type=TourObjectTypes.GPX,
        )
        return gpx.to_xml().encode("utf-8")
    except Exception as exc:  # noqa: BLE001
        logger.error("Failed to download GPX for tour %s: %s", tour_id, exc)
        return None


def export_tours(email: str, password: str, output_dir: Path) -> None:
    logger.info("Authenticating with Komoot …")
    try:
        connector = KomootConnector(email=email, password=password)
    except ConnectionError as exc:
        logger.error("Authentication failed: %s", exc)
        sys.exit(1)

    user_identifier = connector.authentication.get_username()
    logger.info("Logged in as %s.", user_identifier)

    # Create output subfolders (type / visibility)
    for type_folder in SUBFOLDER_MAP.values():
        for status in ALL_STATUSES:
            (output_dir / type_folder / status).mkdir(parents=True, exist_ok=True)

    logger.info("Fetching tour list …")
    tours = fetch_all_tours(connector, user_identifier)

    planned_count = sum(1 for t, _ in tours if t.type == TourTypes.TOUR_PLANNED)
    recorded_count = sum(1 for t, _ in tours if t.type == TourTypes.TOUR_RECORDED)
    total = len(tours)
    logger.info(
        "Found %d unique tours: %d planned, %d made.",
        total,
        planned_count,
        recorded_count,
    )

    success = 0
    skipped_existing = 0
    skipped_type = 0
    failed = 0

    for i, (tour, status) in enumerate(tours, start=1):
        type_folder = SUBFOLDER_MAP.get(tour.type)
        if type_folder is None:
            logger.warning(
                "[%d/%d] Unknown tour type %r for tour %s – skipping.",
                i,
                total,
                tour.type,
                tour.id,
            )
            skipped_type += 1
            continue

        safe_name = sanitize_filename(tour.name)
        date_prefix = tour.start_date.strftime("%Y-%m-%d")
        filename = f"{date_prefix}_{tour.id}_{safe_name}.gpx"
        dest = output_dir / type_folder / status / filename

        if dest.exists():
            logger.info("[%d/%d] Already exists, skipping: %s", i, total, dest.name)
            skipped_existing += 1
            continue

        logger.info(
            "[%d/%d] Downloading %r  [%s / %s]",
            i,
            total,
            tour.name,
            type_folder,
            status,
        )
        gpx_bytes = download_tour_gpx(connector, tour.id)
        if gpx_bytes is None:
            failed += 1
            continue

        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(gpx_bytes)
        success += 1

    logger.info(
        "Done. %d saved, %d skipped (already existed), %d skipped (unknown type), %d failed.",
        success,
        skipped_existing,
        skipped_type,
        failed,
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Export all Komoot tours to GPX files sorted by type."
    )
    parser.add_argument(
        "--email",
        default=os.environ.get("KOMOOT_EMAIL"),
        help="Komoot account e-mail (or set KOMOOT_EMAIL env var).",
    )
    parser.add_argument(
        "--password",
        default=os.environ.get("KOMOOT_PASSWORD"),
        help="Komoot account password (or set KOMOOT_PASSWORD env var). "
        "Passing via flag is visible in the process list; prefer the env var.",
    )
    parser.add_argument(
        "--output-dir",
        default="tours",
        help="Root directory for exported tours (default: ./tours).",
    )
    args = parser.parse_args()

    if not args.email:
        parser.error("Komoot e-mail is required (--email or KOMOOT_EMAIL).")
    if not args.password:
        args.password = getpass.getpass("Komoot password: ")

    export_tours(
        email=args.email,
        password=args.password,
        output_dir=Path(args.output_dir),
    )


if __name__ == "__main__":
    main()
