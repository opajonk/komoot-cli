use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use clap::Parser;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const TOUR_PLANNED: &str = "tour_planned";
const TOUR_RECORDED: &str = "tour_recorded";
const TOUR_TYPES: [&str; 2] = [TOUR_PLANNED, TOUR_RECORDED];
const STATUSES: [&str; 3] = ["public", "friends", "private"];

const USER_LOGIN_URL: &str = "https://api.komoot.de/v006/account/email/{email}/";
const LIST_TOURS_URL: &str = "https://api.komoot.de/v007/users/{user}/tours/";
const TOUR_URL: &str = "https://api.komoot.de/v007/tours/{tour_id}";

#[derive(Parser, Debug)]
#[command(name = "komoot-export")]
#[command(about = "Export all Komoot tours to GPX files sorted by type and visibility.")]
struct Args {
    #[arg(long, env = "KOMOOT_EMAIL")]
    email: Option<String>,

    #[arg(long, env = "KOMOOT_PASSWORD")]
    password: Option<String>,

    #[arg(long, default_value = "tours")]
    output_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TourEntry {
    id: String,
    name: String,
    tour_type: String,
    status: String,
    date: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ExportSummary {
    saved: usize,
    skipped_existing: usize,
    skipped_type: usize,
    failed: usize,
}

trait KomootApi {
    fn username(&self) -> &str;
    fn fetch_all_tours(&self) -> Result<Vec<TourEntry>>;
    fn download_tour_gpx(&self, tour_id: &str) -> Result<Vec<u8>>;
}

struct HttpKomootClient {
    http: Client,
    email: String,
    password: String,
    username: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    username: String,
}

#[derive(Deserialize)]
struct ToursResponse {
    #[serde(rename = "_embedded")]
    embedded: Embedded,
    page: PageInfo,
}

#[derive(Deserialize)]
struct Embedded {
    tours: Vec<RawTour>,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "totalPages")]
    total_pages: usize,
    number: usize,
}

#[derive(Deserialize)]
struct RawTour {
    id: serde_json::Value,
    name: Option<String>,
    #[serde(rename = "type")]
    tour_type: Option<String>,
    status: Option<String>,
    date: Option<String>,
}

impl HttpKomootClient {
    fn authenticate(email: String, password: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent("komoot-export-rust")
            .build()
            .context("failed to build HTTP client")?;
        let login_url = USER_LOGIN_URL.replace("{email}", &email);
        let response = http
            .get(login_url)
            .basic_auth(&email, Some(&password))
            .send()
            .context("failed to connect to Komoot login endpoint")?;

        if response.status() == StatusCode::FORBIDDEN {
            bail!("authentication failed: check credentials");
        }
        if !response.status().is_success() {
            bail!("authentication failed with status {}", response.status());
        }

        let body: LoginResponse = response
            .json()
            .context("failed to parse Komoot login response")?;

        Ok(Self {
            http,
            email,
            password,
            username: body.username,
        })
    }

    fn fetch_page(
        &self,
        user_identifier: &str,
        tour_type: &str,
        status: &str,
        page: usize,
    ) -> Result<ToursResponse> {
        let url = LIST_TOURS_URL.replace("{user}", user_identifier);
        let response = self
            .http
            .get(url)
            .basic_auth(&self.email, Some(&self.password))
            .query(&[
                ("type", tour_type),
                ("status", status),
                ("page", &page.to_string()),
            ])
            .send()
            .context("failed to request tours page")?;

        if response.status() == StatusCode::FORBIDDEN {
            bail!("unauthorized when requesting tours");
        }
        if !response.status().is_success() {
            bail!("failed to request tours, status {}", response.status());
        }

        response
            .json()
            .context("failed to parse tours response JSON")
    }
}

impl KomootApi for HttpKomootClient {
    fn username(&self) -> &str {
        &self.username
    }

    fn fetch_all_tours(&self) -> Result<Vec<TourEntry>> {
        let mut all_tours = Vec::new();
        let mut seen_ids = HashSet::new();

        for status in STATUSES {
            for tour_type in TOUR_TYPES {
                let mut page = 0usize;
                loop {
                    let response = match self.fetch_page(self.username(), tour_type, status, page) {
                        Ok(data) => data,
                        Err(err) => {
                            eprintln!(
                                "Could not fetch tours (type={tour_type}, status={status}, page={page}): {err}"
                            );
                            break;
                        }
                    };

                    for raw_tour in response.embedded.tours {
                        let tour_id = match &raw_tour.id {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        if !seen_ids.insert(tour_id.clone()) {
                            continue;
                        }
                        all_tours.push(TourEntry {
                            id: tour_id,
                            name: raw_tour.name.unwrap_or_else(|| "Unknown".to_string()),
                            tour_type: raw_tour.tour_type.unwrap_or_else(|| tour_type.to_string()),
                            status: raw_tour.status.unwrap_or_else(|| status.to_string()),
                            date: raw_tour.date.unwrap_or_default(),
                        });
                    }

                    if response.page.number + 1 >= response.page.total_pages.max(1) {
                        break;
                    }
                    page = response.page.number + 1;
                }
            }
        }

        Ok(all_tours)
    }

    fn download_tour_gpx(&self, tour_id: &str) -> Result<Vec<u8>> {
        let url = format!("{}.gpx", TOUR_URL.replace("{tour_id}", tour_id));
        let response = self
            .http
            .get(url)
            .basic_auth(&self.email, Some(&self.password))
            .send()
            .context("failed to request GPX")?;

        if response.status() == StatusCode::FORBIDDEN {
            bail!("unauthorized while downloading GPX");
        }
        if response.status() == StatusCode::NOT_FOUND {
            bail!("tour not found");
        }
        if !response.status().is_success() {
            bail!("failed to download GPX with status {}", response.status());
        }

        response
            .bytes()
            .map(|b| b.to_vec())
            .context("failed to read GPX body")
    }
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

fn export_with_client(client: &dyn KomootApi, output_dir: &Path) -> Result<ExportSummary> {
    for folder in ["planned", "made"] {
        for status in STATUSES {
            fs::create_dir_all(output_dir.join(folder).join(status))
                .with_context(|| format!("failed to create output folder for {folder}/{status}"))?;
        }
    }

    let tours = client.fetch_all_tours()?;
    let mut summary = ExportSummary::default();

    for tour in tours {
        let Some(folder) = type_folder(&tour.tour_type) else {
            summary.skipped_type += 1;
            continue;
        };

        let safe_name = sanitize_filename(&tour.name);
        let date_prefix = parse_date_prefix(&tour.date);
        let filename = format!("{date_prefix}_{}_{}.gpx", tour.id, safe_name);
        let destination = output_dir.join(folder).join(&tour.status).join(filename);

        if destination.exists() {
            summary.skipped_existing += 1;
            continue;
        }

        let gpx = match client.download_tour_gpx(&tour.id) {
            Ok(body) => body,
            Err(err) => {
                eprintln!("Failed to download GPX for tour {}: {err}", tour.id);
                summary.failed += 1;
                continue;
            }
        };

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create folder {}", parent.display()))?;
        }
        fs::write(&destination, gpx)
            .with_context(|| format!("failed to write {}", destination.display()))?;
        summary.saved += 1;
    }

    Ok(summary)
}

fn run(args: Args) -> Result<()> {
    let email = args
        .email
        .ok_or_else(|| anyhow!("KOMOOT_EMAIL or --email is required"))?;
    let password = match args.password {
        Some(pwd) => pwd,
        None => rpassword::prompt_password("Komoot password: ")
            .context("failed to read password from terminal")?,
    };

    let client = HttpKomootClient::authenticate(email, password)?;
    println!("Authentication successful.");

    let summary = export_with_client(&client, &args.output_dir)?;
    println!(
        "Done. {} saved, {} skipped (already existed), {} skipped (unknown type), {} failed.",
        summary.saved, summary.skipped_existing, summary.skipped_type, summary.failed
    );
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(err) = run(args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockKomootClient {
        username: String,
        tours: Vec<TourEntry>,
        gpx_by_id: HashMap<String, Option<Vec<u8>>>,
    }

    impl KomootApi for MockKomootClient {
        fn username(&self) -> &str {
            &self.username
        }

        fn fetch_all_tours(&self) -> Result<Vec<TourEntry>> {
            Ok(self.tours.clone())
        }

        fn download_tour_gpx(&self, tour_id: &str) -> Result<Vec<u8>> {
            match self.gpx_by_id.get(tour_id) {
                Some(Some(data)) => Ok(data.clone()),
                Some(None) => bail!("download failed"),
                None => bail!("missing mock entry"),
            }
        }
    }

    fn make_tour(id: &str, name: &str, tour_type: &str, status: &str, date: &str) -> TourEntry {
        TourEntry {
            id: id.to_string(),
            name: name.to_string(),
            tour_type: tour_type.to_string(),
            status: status.to_string(),
            date: date.to_string(),
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

        let summary = export_with_client(&client, tmp.path()).expect("export");
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

        let summary = export_with_client(&client, tmp.path()).expect("export");
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

        let summary = export_with_client(&client, tmp.path()).expect("export");
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

        let summary = export_with_client(&client, tmp.path()).expect("export");
        assert_eq!(summary.failed, 1);
        assert!(
            !tmp.path()
                .join("made/public/2024-01-01_55_Fail_Tour.gpx")
                .exists()
        );
    }
}
