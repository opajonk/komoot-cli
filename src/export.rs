use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::Path;

use crate::api::{KomootApi, STATUSES, TOUR_PLANNED, TOUR_RECORDED};
use crate::filter::{Filters, tour_matches_filters};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExportSummary {
    pub saved: usize,
    pub skipped_existing: usize,
    pub skipped_filter: usize,
    pub skipped_type: usize,
    pub failed: usize,
}

fn sanitize_filename(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .replace(' ', "_")
}

fn parse_date_prefix(raw_date: &str) -> String {
    DateTime::parse_from_rfc3339(raw_date)
        .map(|dt| dt.with_timezone(&Utc).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| "2000-01-01".to_string())
}

fn type_folder(tour_type: &str) -> Option<&'static str> {
    match tour_type {
        TOUR_PLANNED => Some("planned"),
        TOUR_RECORDED => Some("made"),
        _ => None,
    }
}

pub fn export_with_client(
    client: &dyn KomootApi,
    output_dir: &Path,
    filters: &Filters,
) -> Result<ExportSummary> {
    for folder in ["planned", "made"] {
        for status in STATUSES {
            fs::create_dir_all(output_dir.join(folder).join(status))
                .with_context(|| format!("failed to create output folder for {folder}/{status}"))?;
        }
    }

    let tours = client.fetch_all_tours()?;
    let total_tours = tours.len();
    println!("Starting export of {total_tours} tours...");
    let mut summary = ExportSummary::default();
    let log_progress = |processed: usize, summary: &ExportSummary| {
        if processed.is_multiple_of(100) || processed == total_tours {
            println!(
                "Progress: {processed}/{total_tours} processed (saved={}, skipped existing={}, skipped filter={}, skipped unknown type={}, failed={}).",
                summary.saved,
                summary.skipped_existing,
                summary.skipped_filter,
                summary.skipped_type,
                summary.failed
            );
        }
    };

    for (index, tour) in tours.into_iter().enumerate() {
        if let Some(folder) = type_folder(&tour.tour_type) {
            if !tour_matches_filters(&tour, filters) {
                summary.skipped_filter += 1;
            } else {
                let safe_name = sanitize_filename(&tour.name);
                let date_prefix = parse_date_prefix(&tour.date);
                let filename = format!("{date_prefix}_{}_{}.gpx", tour.id, safe_name);
                let destination = output_dir.join(folder).join(&tour.status).join(filename);

                if destination.exists() {
                    summary.skipped_existing += 1;
                } else {
                    match client.download_tour_gpx(&tour.id) {
                        Ok(gpx) => {
                            if let Some(parent) = destination.parent() {
                                fs::create_dir_all(parent).with_context(|| {
                                    format!("failed to create folder {}", parent.display())
                                })?;
                            }
                            fs::write(&destination, gpx).with_context(|| {
                                format!("failed to write {}", destination.display())
                            })?;
                            summary.saved += 1;
                        }
                        Err(err) => {
                            eprintln!("Failed to download GPX for tour {}: {err}", tour.id);
                            summary.failed += 1;
                        }
                    }
                }
            }
        } else {
            summary.skipped_type += 1;
        }

        let processed = index + 1;
        log_progress(processed, &summary);
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_helpers::{MockKomootClient, make_tour};
    use crate::api::{TOUR_PLANNED, TOUR_RECORDED};
    use crate::filter::Filters;
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn make_gpx_client(tours: Vec<crate::api::TourEntry>) -> MockKomootClient {
        let gpx_by_id = tours
            .iter()
            .map(|t| (t.id.clone(), Some(b"<gpx/>".to_vec())))
            .collect();
        MockKomootClient {
            username: "user".to_string(),
            tours,
            gpx_by_id,
        }
    }

    #[test]
    fn sanitize_filename_replaces_unsafe_characters() {
        assert_eq!(sanitize_filename("Tour/2024: #1!"), "Tour_2024___1_");
    }

    #[test]
    fn sanitize_filename_replaces_spaces() {
        assert_eq!(sanitize_filename("Sunday Ride"), "Sunday_Ride");
    }

    #[test]
    fn parse_date_prefix_valid_date() {
        assert_eq!(parse_date_prefix("2024-06-01T10:30:00Z"), "2024-06-01");
    }

    #[test]
    fn parse_date_prefix_invalid_date_fallback() {
        assert_eq!(parse_date_prefix("not-a-date"), "2000-01-01");
    }

    #[test]
    fn export_writes_gpx_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tour = make_tour(
            "99",
            "Test Tour",
            TOUR_RECORDED,
            "public",
            "2024-03-15T00:00:00Z",
        );
        let mut gpx_by_id = HashMap::new();
        gpx_by_id.insert("99".to_string(), Some(b"<gpx/>".to_vec()));
        let client = MockKomootClient {
            username: "user".to_string(),
            tours: vec![tour],
            gpx_by_id,
        };

        let summary = export_with_client(&client, tmp.path(), &Filters::default()).expect("export");
        assert_eq!(summary.saved, 1);
        let expected = tmp.path().join("made/public/2024-03-15_99_Test_Tour.gpx");
        assert!(expected.exists());
        assert_eq!(fs::read(expected).expect("read"), b"<gpx/>".to_vec());
    }

    #[test]
    fn export_skips_existing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let existing_path = tmp.path().join("made/public/2024-03-15_99_Test_Tour.gpx");
        fs::create_dir_all(existing_path.parent().expect("parent")).expect("mkdir");
        fs::write(&existing_path, b"existing").expect("write");

        let tour = make_tour(
            "99",
            "Test Tour",
            TOUR_RECORDED,
            "public",
            "2024-03-15T00:00:00Z",
        );
        let mut gpx_by_id = HashMap::new();
        gpx_by_id.insert("99".to_string(), Some(b"<gpx/>".to_vec()));
        let client = MockKomootClient {
            username: "user".to_string(),
            tours: vec![tour],
            gpx_by_id,
        };

        let summary = export_with_client(&client, tmp.path(), &Filters::default()).expect("export");
        assert_eq!(summary.saved, 0);
        assert_eq!(summary.skipped_existing, 1);
        assert_eq!(fs::read(existing_path).expect("read"), b"existing".to_vec());
    }

    #[test]
    fn export_skips_unknown_type() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tour = make_tour(
            "77",
            "Unknown",
            "tour_unknown",
            "public",
            "2024-03-15T00:00:00Z",
        );
        let client = MockKomootClient {
            username: "user".to_string(),
            tours: vec![tour],
            gpx_by_id: HashMap::new(),
        };

        let summary = export_with_client(&client, tmp.path(), &Filters::default()).expect("export");
        assert_eq!(summary.saved, 0);
        assert_eq!(summary.skipped_type, 1);
    }

    #[test]
    fn export_marks_failed_download() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tour = make_tour(
            "55",
            "Fail Tour",
            TOUR_RECORDED,
            "public",
            "2024-01-01T00:00:00Z",
        );
        let mut gpx_by_id = HashMap::new();
        gpx_by_id.insert("55".to_string(), None);
        let client = MockKomootClient {
            username: "user".to_string(),
            tours: vec![tour],
            gpx_by_id,
        };

        let summary = export_with_client(&client, tmp.path(), &Filters::default()).expect("export");
        assert_eq!(summary.failed, 1);
        assert!(
            !tmp.path()
                .join("made/public/2024-01-01_55_Fail_Tour.gpx")
                .exists()
        );
    }

    #[test]
    fn export_unknown_type_counts_as_skipped_type_even_with_type_filter() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tour = make_tour(
            "88",
            "Unknown",
            "tour_unknown",
            "public",
            "2024-03-15T00:00:00Z",
        );
        let client = MockKomootClient {
            username: "user".to_string(),
            tours: vec![tour],
            gpx_by_id: HashMap::new(),
        };
        let filters = Filters {
            types: Some(["recorded".to_string()].into()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.skipped_type, 1);
        assert_eq!(summary.skipped_filter, 0);
    }

    #[test]
    fn filter_status_skips_non_matching_tours() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tours = vec![
            make_tour(
                "1",
                "Public",
                TOUR_RECORDED,
                "public",
                "2024-01-01T00:00:00Z",
            ),
            make_tour(
                "2",
                "Private",
                TOUR_RECORDED,
                "private",
                "2024-01-02T00:00:00Z",
            ),
            make_tour(
                "3",
                "Friends",
                TOUR_RECORDED,
                "friends",
                "2024-01-03T00:00:00Z",
            ),
        ];
        let client = make_gpx_client(tours);
        let filters = Filters {
            statuses: Some(["public".to_string()].into()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.skipped_filter, 2);
        assert!(
            tmp.path()
                .join("made/public/2024-01-01_1_Public.gpx")
                .exists()
        );
    }

    #[test]
    fn filter_type_skips_non_matching_tours() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tours = vec![
            make_tour(
                "10",
                "Planned",
                TOUR_PLANNED,
                "public",
                "2024-02-01T00:00:00Z",
            ),
            make_tour(
                "11",
                "Recorded",
                TOUR_RECORDED,
                "public",
                "2024-02-02T00:00:00Z",
            ),
        ];
        let client = make_gpx_client(tours);
        let filters = Filters {
            types: Some(["recorded".to_string()].into()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.skipped_filter, 1);
        assert!(
            tmp.path()
                .join("made/public/2024-02-02_11_Recorded.gpx")
                .exists()
        );
    }

    #[test]
    fn filter_from_date_skips_earlier_tours() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tours = vec![
            make_tour("20", "Old", TOUR_RECORDED, "public", "2023-12-31T00:00:00Z"),
            make_tour("21", "New", TOUR_RECORDED, "public", "2024-01-01T00:00:00Z"),
        ];
        let client = make_gpx_client(tours);
        let filters = Filters {
            from_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.skipped_filter, 1);
        assert!(
            tmp.path()
                .join("made/public/2024-01-01_21_New.gpx")
                .exists()
        );
    }

    #[test]
    fn filter_to_date_skips_later_tours() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tours = vec![
            make_tour(
                "30",
                "Early",
                TOUR_RECORDED,
                "public",
                "2024-01-01T00:00:00Z",
            ),
            make_tour(
                "31",
                "Late",
                TOUR_RECORDED,
                "public",
                "2024-06-01T00:00:00Z",
            ),
        ];
        let client = make_gpx_client(tours);
        let filters = Filters {
            to_date: Some(NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.skipped_filter, 1);
        assert!(
            tmp.path()
                .join("made/public/2024-01-01_30_Early.gpx")
                .exists()
        );
    }

    #[test]
    fn filter_date_range_keeps_only_tours_in_range() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tours = vec![
            make_tour(
                "40",
                "Before",
                TOUR_RECORDED,
                "public",
                "2023-12-31T00:00:00Z",
            ),
            make_tour(
                "41",
                "Inside",
                TOUR_RECORDED,
                "public",
                "2024-03-01T00:00:00Z",
            ),
            make_tour(
                "42",
                "After",
                TOUR_RECORDED,
                "public",
                "2024-07-01T00:00:00Z",
            ),
        ];
        let client = make_gpx_client(tours);
        let filters = Filters {
            from_date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            to_date: Some(NaiveDate::from_ymd_opt(2024, 6, 30).unwrap()),
            ..Filters::default()
        };
        let summary = export_with_client(&client, tmp.path(), &filters).expect("export");
        assert_eq!(summary.saved, 1);
        assert_eq!(summary.skipped_filter, 2);
    }
}
