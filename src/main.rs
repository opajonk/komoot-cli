use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
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

    /// Only export tours on or after this date (YYYY-MM-DD)
    #[arg(long, value_name = "YYYY-MM-DD")]
    from_date: Option<String>,

    /// Only export tours on or before this date (YYYY-MM-DD)
    #[arg(long, value_name = "YYYY-MM-DD")]
    to_date: Option<String>,

    /// Only export tours with the given visibility; comma-separated (public,friends,private)
    #[arg(long, value_delimiter = ',')]
    status: Vec<String>,

    /// Only export tours of the given type; comma-separated (planned,recorded)
    #[arg(long = "type", value_delimiter = ',')]
    tour_type: Vec<String>,
}

#[derive(Debug, Default)]
struct Filters {
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
    statuses: Option<HashSet<String>>,
    /// Values are "planned" or "recorded"
    types: Option<HashSet<String>>,
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
    skipped_filter: usize,
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
    endpoints: KomootEndpoints,
}

#[derive(Clone)]
struct KomootEndpoints {
    user_login_url: String,
    list_tours_url: String,
    tour_url: String,
}

impl Default for KomootEndpoints {
    fn default() -> Self {
        Self {
            user_login_url: USER_LOGIN_URL.to_string(),
            list_tours_url: LIST_TOURS_URL.to_string(),
            tour_url: TOUR_URL.to_string(),
        }
    }
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
        Self::authenticate_with_endpoints(email, password, KomootEndpoints::default())
    }

    fn authenticate_with_endpoints(
        email: String,
        password: String,
        endpoints: KomootEndpoints,
    ) -> Result<Self> {
        let http = Client::builder()
            .user_agent("komoot-export-rust")
            .build()
            .context("failed to build HTTP client")?;
        let login_url = endpoints.user_login_url.replace("{email}", &email);
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
            endpoints,
        })
    }

    fn fetch_page(
        &self,
        user_identifier: &str,
        tour_type: &str,
        status: &str,
        page: usize,
    ) -> Result<ToursResponse> {
        let url = self
            .endpoints
            .list_tours_url
            .replace("{user}", user_identifier);
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
        println!("Fetching tour lists...");
        let mut all_tours = Vec::new();
        let mut seen_ids = HashSet::new();

        for status in STATUSES {
            for tour_type in TOUR_TYPES {
                println!("Loading tours (type={tour_type}, status={status})...");
                let before_count = all_tours.len();
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
                let added = all_tours.len() - before_count;
                println!("Loaded tours (type={tour_type}, status={status}): {added} new entries.");
            }
        }

        Ok(all_tours)
    }

    fn download_tour_gpx(&self, tour_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}.gpx",
            self.endpoints.tour_url.replace("{tour_id}", tour_id)
        );
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

fn tour_matches_filters(tour: &TourEntry, filters: &Filters) -> bool {
    if let Some(ref types) = filters.types {
        let label = match tour.tour_type.as_str() {
            TOUR_PLANNED => "planned",
            TOUR_RECORDED => "recorded",
            _ => "",
        };
        if !types.contains(label) {
            return false;
        }
    }
    if let Some(ref statuses) = filters.statuses
        && !statuses.contains(&tour.status)
    {
        return false;
    }
    // parse_date_prefix always returns a valid YYYY-MM-DD string (falling back to
    // "2000-01-01" for unparseable tour dates), so NaiveDate parsing here will not fail.
    let tour_date =
        NaiveDate::parse_from_str(&parse_date_prefix(&tour.date), "%Y-%m-%d")
            .expect("parse_date_prefix guarantees a valid YYYY-MM-DD output");
    if let Some(from) = filters.from_date
        && tour_date < from
    {
        return false;
    }
    if let Some(to) = filters.to_date
        && tour_date > to
    {
        return false;
    }
    true
}

fn build_filters(args: &Args) -> Result<Filters> {
    let from_date = args
        .from_date
        .as_deref()
        .map(|s| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("invalid --from-date '{s}': expected YYYY-MM-DD"))
        })
        .transpose()?;
    let to_date = args
        .to_date
        .as_deref()
        .map(|s| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .with_context(|| format!("invalid --to-date '{s}': expected YYYY-MM-DD"))
        })
        .transpose()?;
    for s in &args.status {
        if !["public", "friends", "private"].contains(&s.as_str()) {
            bail!("invalid --status value '{s}': must be one of public, friends, private");
        }
    }
    for t in &args.tour_type {
        if !["planned", "recorded"].contains(&t.as_str()) {
            bail!("invalid --type value '{t}': must be one of planned, recorded");
        }
    }
    Ok(Filters {
        from_date,
        to_date,
        statuses: if args.status.is_empty() {
            None
        } else {
            Some(args.status.iter().cloned().collect())
        },
        types: if args.tour_type.is_empty() {
            None
        } else {
            Some(args.tour_type.iter().cloned().collect())
        },
    })
}

fn export_with_client(
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
        if !tour_matches_filters(&tour, filters) {
            summary.skipped_filter += 1;
        } else if let Some(folder) = type_folder(&tour.tour_type) {
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
        } else {
            summary.skipped_type += 1;
        }

        let processed = index + 1;
        log_progress(processed, &summary);
    }

    Ok(summary)
}

fn run(args: Args) -> Result<()> {
    let filters = build_filters(&args)?;
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

    let summary = export_with_client(&client, &args.output_dir, &filters)?;
    println!(
        "Done. {} saved, {} skipped (already existed), {} skipped (filter), {} skipped (unknown type), {} failed.",
        summary.saved,
        summary.skipped_existing,
        summary.skipped_filter,
        summary.skipped_type,
        summary.failed
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

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

    fn parse_query(path: &str) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Some((_, query)) = path.split_once('?') else {
            return out;
        };
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            out.insert(key.to_string(), value.to_string());
        }
        out
    }

    fn read_request(stream: &mut TcpStream) -> (String, String, HashMap<String, String>) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("read request line");

        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        (method, path, headers)
    }

    fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
        let status_text = match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        write!(
            stream,
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("write response");
    }

    fn start_mock_server<F>(
        expected_requests: usize,
        handler: F,
    ) -> (String, thread::JoinHandle<()>)
    where
        F: Fn(usize, &str, &str, &HashMap<String, String>) -> (u16, &'static str, String)
            + Send
            + Sync
            + 'static,
    {
        let (addr_tx, addr_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("local addr");
            addr_tx.send(addr).expect("send addr");
            for idx in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("accept");
                let (method, path, headers) = read_request(&mut stream);
                let (status, content_type, body) = handler(idx, &method, &path, &headers);
                write_response(&mut stream, status, content_type, &body);
            }
        });
        let addr = addr_rx.recv().expect("recv addr");
        (format!("http://{addr}"), handle)
    }

    fn make_endpoints(base_url: &str) -> KomootEndpoints {
        KomootEndpoints {
            user_login_url: format!("{base_url}/v006/account/email/{{email}}/"),
            list_tours_url: format!("{base_url}/v007/users/{{user}}/tours/"),
            tour_url: format!("{base_url}/v007/tours/{{tour_id}}"),
        }
    }

    fn make_http_client(
        username: &str,
        email: &str,
        password: String,
        endpoints: KomootEndpoints,
    ) -> HttpKomootClient {
        HttpKomootClient {
            http: Client::builder()
                .user_agent("komoot-export-rust-test")
                .build()
                .expect("build client"),
            email: email.to_string(),
            password,
            username: username.to_string(),
            endpoints,
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
    fn authenticate_requests_login_endpoint_and_parses_username() {
        let (base_url, handle) = start_mock_server(1, |_, method, path, headers| {
            assert_eq!(method, "GET");
            assert_eq!(path, "/v006/account/email/test@example.com/");
            assert!(
                headers
                    .get("authorization")
                    .is_some_and(|value| value.starts_with("Basic "))
            );
            (
                200,
                "application/json",
                r#"{"username":"demo-user"}"#.to_string(),
            )
        });
        let password = format!("pw-{}", std::process::id());

        let client = HttpKomootClient::authenticate_with_endpoints(
            "test@example.com".to_string(),
            password,
            make_endpoints(&base_url),
        )
        .expect("authenticate");

        assert_eq!(client.username(), "demo-user");
        handle.join().expect("join server");
    }

    #[test]
    fn fetch_all_tours_handles_pagination_and_deduplicates_ids() {
        let (base_url, handle) = start_mock_server(7, |_, method, path, headers| {
            assert_eq!(method, "GET");
            assert!(path.starts_with("/v007/users/demo/tours/"));
            assert!(
                headers
                    .get("authorization")
                    .is_some_and(|value| value.starts_with("Basic "))
            );

            let query = parse_query(path);
            let tour_type = query.get("type").map(String::as_str).unwrap_or_default();
            let status = query.get("status").map(String::as_str).unwrap_or_default();
            let page = query.get("page").map(String::as_str).unwrap_or_default();

            let body = if tour_type == TOUR_RECORDED && status == "public" && page == "0" {
                r#"{
                    "_embedded": {
                        "tours": [
                            {"id":"1","name":"First","type":"tour_recorded","status":"public","date":"2024-01-01T00:00:00Z"}
                        ]
                    },
                    "page": {"totalPages": 2, "number": 0}
                }"#
            } else if tour_type == TOUR_RECORDED && status == "public" && page == "1" {
                r#"{
                    "_embedded": {
                        "tours": [
                            {"id":"1","name":"First duplicate","type":"tour_recorded","status":"public","date":"2024-01-01T00:00:00Z"},
                            {"id":"2","name":"Second","type":"tour_recorded","status":"public","date":"2024-01-02T00:00:00Z"}
                        ]
                    },
                    "page": {"totalPages": 2, "number": 1}
                }"#
            } else {
                r#"{
                    "_embedded": {"tours": []},
                    "page": {"totalPages": 1, "number": 0}
                }"#
            };

            (200, "application/json", body.to_string())
        });

        let client = make_http_client(
            "demo",
            "test@example.com",
            format!("pw-{}", std::process::id()),
            make_endpoints(&base_url),
        );
        let tours = client.fetch_all_tours().expect("fetch all tours");

        assert_eq!(tours.len(), 2);
        assert_eq!(tours.iter().filter(|tour| tour.id == "1").count(), 1);
        assert!(tours.iter().any(|tour| tour.id == "1"));
        assert!(tours.iter().any(|tour| tour.id == "2"));
        handle.join().expect("join server");
    }

    #[test]
    fn download_tour_gpx_requests_gpx_endpoint_without_query_and_returns_body() {
        let (base_url, handle) = start_mock_server(1, |_, method, path, headers| {
            assert_eq!(method, "GET");
            assert_eq!(path, "/v007/tours/42.gpx");
            assert!(
                headers
                    .get("authorization")
                    .is_some_and(|value| value.starts_with("Basic "))
            );
            (200, "application/gpx+xml", "<gpx>ok</gpx>".to_string())
        });

        let client = make_http_client(
            "demo",
            "test@example.com",
            format!("pw-{}", std::process::id()),
            make_endpoints(&base_url),
        );
        let body = client.download_tour_gpx("42").expect("download gpx");

        assert_eq!(body, b"<gpx>ok</gpx>".to_vec());
        handle.join().expect("join server");
    }

    fn make_gpx_client(tours: Vec<TourEntry>) -> MockKomootClient {
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

    #[test]
    fn build_filters_rejects_invalid_date() {
        let args = Args {
            email: None,
            password: None,
            output_dir: PathBuf::from("tours"),
            from_date: Some("not-a-date".to_string()),
            to_date: None,
            status: vec![],
            tour_type: vec![],
        };
        assert!(build_filters(&args).is_err());
    }

    #[test]
    fn build_filters_rejects_invalid_status() {
        let args = Args {
            email: None,
            password: None,
            output_dir: PathBuf::from("tours"),
            from_date: None,
            to_date: None,
            status: vec!["unknown".to_string()],
            tour_type: vec![],
        };
        assert!(build_filters(&args).is_err());
    }

    #[test]
    fn build_filters_rejects_invalid_type() {
        let args = Args {
            email: None,
            password: None,
            output_dir: PathBuf::from("tours"),
            from_date: None,
            to_date: None,
            status: vec![],
            tour_type: vec!["cycling".to_string()],
        };
        assert!(build_filters(&args).is_err());
    }

    #[test]
    fn build_filters_accepts_valid_inputs() {
        let args = Args {
            email: None,
            password: None,
            output_dir: PathBuf::from("tours"),
            from_date: Some("2024-01-01".to_string()),
            to_date: Some("2024-12-31".to_string()),
            status: vec!["public".to_string(), "friends".to_string()],
            tour_type: vec!["recorded".to_string()],
        };
        let filters = build_filters(&args).expect("valid filters");
        assert_eq!(
            filters.from_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        );
        assert_eq!(
            filters.to_date,
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())
        );
        assert_eq!(
            filters.statuses,
            Some(["public".to_string(), "friends".to_string()].into())
        );
        assert_eq!(filters.types, Some(["recorded".to_string()].into()));
    }
}
