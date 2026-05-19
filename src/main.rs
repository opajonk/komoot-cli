mod api;
mod export;
mod filter;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use std::path::PathBuf;

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

fn run(args: Args) -> Result<()> {
    let filters = filter::build_filters(
        args.from_date.as_deref(),
        args.to_date.as_deref(),
        &args.status,
        &args.tour_type,
    )?;
    let email = args
        .email
        .ok_or_else(|| anyhow!("KOMOOT_EMAIL or --email is required"))?;
    let password = match args.password {
        Some(pwd) => pwd,
        None => rpassword::prompt_password("Komoot password: ")
            .context("failed to read password from terminal")?,
    };

    let client = api::HttpKomootClient::authenticate(email, password)?;
    println!("Authentication successful.");

    let summary = export::export_with_client(&client, &args.output_dir, &filters)?;
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
