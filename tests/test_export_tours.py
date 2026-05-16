"""Unit tests for export_tours.py."""

import sys
from datetime import datetime, timezone
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

import pytest

# ---------------------------------------------------------------------------
# Helpers – make the module importable without kompy installed in the test env
# ---------------------------------------------------------------------------

# Stub out external kompy imports with real string values so that module-level
# constants (SUBFOLDER_MAP, ALL_STATUSES, …) resolve to the correct strings.


def _make_tour_constants_stub():
    mod = MagicMock()
    mod.TourTypes.TOUR_PLANNED = "tour_planned"
    mod.TourTypes.TOUR_RECORDED = "tour_recorded"
    return mod


def _make_privacy_status_stub():
    mod = MagicMock()
    mod.PrivacyStatus.PUBLIC = "public"
    mod.PrivacyStatus.FRIENDS = "friends"
    mod.PrivacyStatus.PRIVATE = "private"
    return mod


def _make_tour_object_types_stub():
    mod = MagicMock()
    mod.TourObjectTypes.GPX = "gpx"
    mod.TourObjectTypes.FIT = "fit"
    mod.TourObjectTypes.KOMPY = "kompy"
    return mod


def _make_query_parameters_stub():
    mod = MagicMock()
    mod.TourQueryParameters.TYPE = "type"
    mod.TourQueryParameters.STATUS = "status"
    mod.TourQueryParameters.PAGE = "page"
    return mod


kompy_stub = MagicMock()
kompy_stub.KomootConnector = MagicMock

sys.modules.setdefault("kompy", kompy_stub)
sys.modules.setdefault("kompy.constants", MagicMock())
sys.modules.setdefault("kompy.constants.tour_constants", _make_tour_constants_stub())
sys.modules.setdefault("kompy.constants.privacy_status", _make_privacy_status_stub())
sys.modules.setdefault(
    "kompy.constants.tour_object_types", _make_tour_object_types_stub()
)
sys.modules.setdefault(
    "kompy.constants.query_parameters", _make_query_parameters_stub()
)

import export_tours  # noqa: E402  (must come after stubs)
from export_tours import (  # noqa: E402
    ALL_STATUSES,
    SUBFOLDER_MAP,
    _parse_date,
    download_tour_gpx,
    export_tours as run_export,
    fetch_all_tours,
    sanitize_filename,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def _make_tour(
    tour_id: str = "123",
    name: str = "My Tour",
    tour_type: str = "tour_recorded",
    date: datetime | None = None,
    status: str = "private",
) -> SimpleNamespace:
    """Return a minimal Tour-like object."""
    return SimpleNamespace(
        id=tour_id,
        name=name,
        type=tour_type,
        status=status,
        start_date=date or datetime(2024, 6, 1, tzinfo=timezone.utc),
    )


# ---------------------------------------------------------------------------
# sanitize_filename
# ---------------------------------------------------------------------------


class TestSanitizeFilename:
    def test_plain_name_unchanged(self):
        assert sanitize_filename("Sunday Ride") == "Sunday_Ride"

    def test_spaces_replaced_by_underscores(self):
        assert " " not in sanitize_filename("a b c")

    def test_special_chars_replaced(self):
        result = sanitize_filename("Tour/2024: #1!")
        assert "/" not in result
        assert ":" not in result
        assert "#" not in result
        assert "!" not in result

    def test_safe_chars_preserved(self):
        result = sanitize_filename("My-Tour_2024.gpx")
        assert result == "My-Tour_2024.gpx"

    def test_empty_string(self):
        assert sanitize_filename("") == ""

    def test_leading_trailing_spaces_stripped(self):
        result = sanitize_filename("  hello  ")
        assert not result.startswith("_")
        assert not result.endswith("_")


# ---------------------------------------------------------------------------
# _parse_date
# ---------------------------------------------------------------------------


class TestParseDate:
    def test_valid_utc_date(self):
        result = _parse_date("2024-06-01T10:30:00Z")
        assert result.year == 2024
        assert result.month == 6
        assert result.day == 1

    def test_valid_offset_date(self):
        result = _parse_date("2024-03-15T08:00:00+02:00")
        assert result.year == 2024

    def test_empty_string_returns_fallback(self):
        result = _parse_date("")
        assert result.year == 2000

    def test_invalid_string_returns_fallback(self):
        result = _parse_date("not-a-date")
        assert result.year == 2000


# ---------------------------------------------------------------------------
# fetch_all_tours helpers
# ---------------------------------------------------------------------------


def _page_response(tours_raw=None, total_pages=1, number=0):
    """Build a mock _get_page_of_tours() response."""
    resp = MagicMock()
    resp.json.return_value = {
        "_embedded": {"tours": tours_raw or []},
        "page": {"totalPages": total_pages, "number": number},
    }
    return resp


def _raw_tour(
    tour_id="123",
    name="My Tour",
    tour_type="tour_recorded",
    status="private",
    date="2024-06-01T00:00:00Z",
):
    return {
        "id": tour_id,
        "name": name,
        "type": tour_type,
        "status": status,
        "date": date,
    }


# ---------------------------------------------------------------------------
# fetch_all_tours
# ---------------------------------------------------------------------------


class TestFetchAllTours:
    def _make_connector(self, side_effects: dict | None = None) -> MagicMock:
        """side_effects maps (tour_type, status) → list of raw tour dicts."""
        connector = MagicMock()
        if side_effects is None:
            connector._get_page_of_tours.return_value = _page_response()
        else:

            def _side_effect(query_parameters, user_identifier):
                t = query_parameters.get("type")
                s = query_parameters.get("status")
                return _page_response(side_effects.get((t, s), []))

            connector._get_page_of_tours.side_effect = _side_effect
        return connector

    def test_returns_empty_list_when_no_tours(self):
        connector = self._make_connector()
        assert fetch_all_tours(connector, "user1") == []

    def test_returns_tour_with_its_actual_status(self):
        td = _raw_tour(tour_id="123", tour_type="tour_recorded", status="private")
        connector = self._make_connector(
            side_effects={("tour_recorded", "public"): [td]}
        )
        result = fetch_all_tours(connector, "user1")
        assert len(result) == 1
        returned_tour, returned_status = result[0]
        assert returned_tour.id == "123"
        # status comes from the API response field, not the query bucket
        assert returned_status == "private"

    def test_actual_status_beats_query_status(self):
        """A tour returned by the 'public' query but marked 'friends' on the
        object must be stored under 'friends'."""
        td = _raw_tour(tour_id="x", tour_type="tour_recorded", status="friends")
        connector = self._make_connector(
            side_effects={("tour_recorded", "public"): [td]}
        )
        result = fetch_all_tours(connector, "user1")
        _, returned_status = result[0]
        assert returned_status == "friends"

    def test_deduplicates_across_status_calls(self):
        """Same tour ID appearing under multiple statuses must appear only once."""
        td = _raw_tour(tour_id="dup", tour_type="tour_recorded", status="private")
        connector = self._make_connector(
            side_effects={
                ("tour_recorded", "public"): [td],
                ("tour_recorded", "friends"): [td],
            }
        )
        result = fetch_all_tours(connector, "user1")
        assert sum(1 for t, _ in result if t.id == "dup") == 1

    def test_covers_all_statuses_and_types(self):
        connector = self._make_connector()
        fetch_all_tours(connector, "user1")
        calls = connector._get_page_of_tours.call_args_list
        statuses = {c.kwargs["query_parameters"].get("status") for c in calls}
        types_queried = {c.kwargs["query_parameters"].get("type") for c in calls}
        for s in ALL_STATUSES:
            assert s in statuses
        for t in SUBFOLDER_MAP:
            assert t in types_queried

    def test_connector_error_is_caught_and_skipped(self):
        connector = MagicMock()
        connector._get_page_of_tours.side_effect = ConnectionError("network")
        # Should not raise; returns empty list.
        result = fetch_all_tours(connector, "user1")
        assert result == []

    def test_multiple_tours_across_types(self):
        planned = _raw_tour(tour_id="p1", tour_type="tour_planned", status="private")
        recorded = _raw_tour(tour_id="r1", tour_type="tour_recorded", status="private")
        connector = self._make_connector(
            side_effects={
                ("tour_planned", "public"): [planned],
                ("tour_recorded", "public"): [recorded],
            }
        )
        result = fetch_all_tours(connector, "user1")
        ids = {t.id for t, _ in result}
        assert "p1" in ids
        assert "r1" in ids

    def test_tour_fields_mapped_correctly(self):
        td = _raw_tour(
            tour_id="99",
            name="Great Ride",
            tour_type="tour_planned",
            status="public",
            date="2024-08-10T12:00:00Z",
        )
        connector = self._make_connector(
            side_effects={("tour_planned", "public"): [td]}
        )
        result = fetch_all_tours(connector, "user1")
        tour, status = result[0]
        assert tour.id == "99"
        assert tour.name == "Great Ride"
        assert tour.type == "tour_planned"
        assert tour.start_date.year == 2024
        assert tour.start_date.month == 8
        assert status == "public"


# ---------------------------------------------------------------------------
# download_tour_gpx
# ---------------------------------------------------------------------------


class TestDownloadTourGpx:
    def test_returns_encoded_gpx_on_success(self):
        gpx_mock = MagicMock()
        gpx_mock.to_xml.return_value = "<gpx/>"
        connector = MagicMock()
        connector.get_tour_by_id.return_value = gpx_mock

        result = download_tour_gpx(connector, "42")

        assert result == b"<gpx/>"
        connector.get_tour_by_id.assert_called_once_with(
            tour_identifier="42",
            object_type=export_tours.TourObjectTypes.GPX,
        )

    def test_returns_none_on_connection_error(self):
        connector = MagicMock()
        connector.get_tour_by_id.side_effect = ConnectionError("fail")
        assert download_tour_gpx(connector, "42") is None

    def test_returns_none_on_value_error(self):
        connector = MagicMock()
        connector.get_tour_by_id.side_effect = ValueError("not found")
        assert download_tour_gpx(connector, "42") is None


# ---------------------------------------------------------------------------
# export_tours (integration-style, filesystem interactions)
# ---------------------------------------------------------------------------


class TestRunExport:
    def test_creates_subfolders(self, tmp_path):
        _make_tour(tour_type="tour_recorded")
        gpx_mock = MagicMock()
        gpx_mock.to_xml.return_value = "<gpx/>"

        connector = MagicMock()
        connector.get_tours.return_value = []

        with (
            patch("export_tours.KomootConnector", return_value=connector),
            patch("export_tours.fetch_all_tours", return_value=[]),
        ):
            run_export("a@b.com", "pw", tmp_path)

        for type_folder in SUBFOLDER_MAP.values():
            for status in ALL_STATUSES:
                assert (tmp_path / type_folder / status).is_dir()

    def test_writes_gpx_file(self, tmp_path):
        tour = _make_tour(
            tour_id="99",
            name="Test Tour",
            tour_type="tour_recorded",
            date=datetime(2024, 3, 15, tzinfo=timezone.utc),
        )

        connector = MagicMock()

        with (
            patch("export_tours.KomootConnector", return_value=connector),
            patch("export_tours.fetch_all_tours", return_value=[(tour, "public")]),
            patch("export_tours.download_tour_gpx", return_value=b"<gpx/>"),
        ):
            run_export("a@b.com", "pw", tmp_path)

        expected = tmp_path / "made" / "public" / "2024-03-15_99_Test_Tour.gpx"
        assert expected.exists()
        assert expected.read_bytes() == b"<gpx/>"

    def test_skips_existing_file(self, tmp_path):
        tour = _make_tour(
            tour_id="99",
            name="Test Tour",
            tour_type="tour_recorded",
            date=datetime(2024, 3, 15, tzinfo=timezone.utc),
        )
        dest = tmp_path / "made" / "public" / "2024-03-15_99_Test_Tour.gpx"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(b"existing")

        connector = MagicMock()
        download_mock = MagicMock(return_value=b"<gpx/>")

        with (
            patch("export_tours.KomootConnector", return_value=connector),
            patch("export_tours.fetch_all_tours", return_value=[(tour, "public")]),
            patch("export_tours.download_tour_gpx", download_mock),
        ):
            run_export("a@b.com", "pw", tmp_path)

        download_mock.assert_not_called()
        assert dest.read_bytes() == b"existing"

    def test_skips_tour_with_unknown_type(self, tmp_path):
        tour = _make_tour(tour_id="77", tour_type="tour_unknown")
        connector = MagicMock()
        download_mock = MagicMock()

        with (
            patch("export_tours.KomootConnector", return_value=connector),
            patch("export_tours.fetch_all_tours", return_value=[(tour, "public")]),
            patch("export_tours.download_tour_gpx", download_mock),
        ):
            run_export("a@b.com", "pw", tmp_path)

        download_mock.assert_not_called()

    def test_skips_tour_when_gpx_download_fails(self, tmp_path):
        tour = _make_tour(
            tour_id="55",
            name="Fail Tour",
            tour_type="tour_recorded",
            date=datetime(2024, 1, 1, tzinfo=timezone.utc),
        )
        connector = MagicMock()

        with (
            patch("export_tours.KomootConnector", return_value=connector),
            patch("export_tours.fetch_all_tours", return_value=[(tour, "public")]),
            patch("export_tours.download_tour_gpx", return_value=None),
        ):
            run_export("a@b.com", "pw", tmp_path)

        assert not (
            tmp_path / "made" / "public" / "2024-01-01_55_Fail_Tour.gpx"
        ).exists()

    def test_auth_failure_exits(self, tmp_path):
        with (
            patch(
                "export_tours.KomootConnector", side_effect=ConnectionError("bad creds")
            ),
            pytest.raises(SystemExit),
        ):
            run_export("a@b.com", "wrong", tmp_path)
