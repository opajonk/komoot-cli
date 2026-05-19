use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashSet;

pub const TOUR_PLANNED: &str = "tour_planned";
pub const TOUR_RECORDED: &str = "tour_recorded";
pub const TOUR_TYPES: [&str; 2] = [TOUR_PLANNED, TOUR_RECORDED];
pub const STATUSES: [&str; 3] = ["public", "friends", "private"];

const USER_LOGIN_URL: &str = "https://api.komoot.de/v006/account/email/{email}/";
const LIST_TOURS_URL: &str = "https://api.komoot.de/v007/users/{user}/tours/";
const TOUR_URL: &str = "https://api.komoot.de/v007/tours/{tour_id}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TourEntry {
    pub id: String,
    pub name: String,
    pub tour_type: String,
    pub status: String,
    pub date: String,
}

pub trait KomootApi {
    fn username(&self) -> &str;
    fn fetch_all_tours(&self) -> Result<Vec<TourEntry>>;
    fn download_tour_gpx(&self, tour_id: &str) -> Result<Vec<u8>>;
}

pub struct HttpKomootClient {
    http: Client,
    email: String,
    password: String,
    username: String,
    endpoints: KomootEndpoints,
}

#[derive(Clone)]
pub struct KomootEndpoints {
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
    pub fn authenticate(email: String, password: String) -> Result<Self> {
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

// Shared test helpers used by api, export, and filter test modules.
#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    pub(crate) struct MockKomootClient {
        pub(crate) username: String,
        pub(crate) tours: Vec<TourEntry>,
        pub(crate) gpx_by_id: HashMap<String, Option<Vec<u8>>>,
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

    pub(crate) fn make_tour(
        id: &str,
        name: &str,
        tour_type: &str,
        status: &str,
        date: &str,
    ) -> TourEntry {
        TourEntry {
            id: id.to_string(),
            name: name.to_string(),
            tour_type: tour_type.to_string(),
            status: status.to_string(),
            date: date.to_string(),
        }
    }

    pub(crate) fn parse_query(path: &str) -> HashMap<String, String> {
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

    pub(crate) fn start_mock_server<F>(
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

    pub(crate) fn make_endpoints(base_url: &str) -> KomootEndpoints {
        KomootEndpoints {
            user_login_url: format!("{base_url}/v006/account/email/{{email}}/"),
            list_tours_url: format!("{base_url}/v007/users/{{user}}/tours/"),
            tour_url: format!("{base_url}/v007/tours/{{tour_id}}"),
        }
    }

    pub(crate) fn make_http_client(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_helpers::*;

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
}
