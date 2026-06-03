mod api;
mod export;
mod filter;
mod list;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "komoot-cli")]
#[command(about = "A CLI for interacting with Komoot.")]
struct Cli {
    #[arg(long, env = "KOMOOT_EMAIL", global = true)]
    email: Option<String>,

    #[arg(long, env = "KOMOOT_PASSWORD", global = true)]
    password: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Commands related to routes
    Routes {
        #[command(subcommand)]
        command: RoutesCommands,
    },
}

#[derive(Subcommand, Debug)]
enum RoutesCommands {
    /// Export all Komoot tours to GPX files sorted by type and visibility
    Export(ExportArgs),
    /// List Komoot tours as a Markdown table
    List(ListArgs),
}

#[derive(clap::Args, Debug)]
struct FilterArgs {
    /// Only include tours on or after this date (YYYY-MM-DD)
    #[arg(long, value_name = "YYYY-MM-DD")]
    from_date: Option<String>,

    /// Only include tours on or before this date (YYYY-MM-DD)
    #[arg(long, value_name = "YYYY-MM-DD")]
    to_date: Option<String>,

    /// Only include tours with the given visibility; comma-separated (public,friends,private)
    #[arg(long, value_delimiter = ',')]
    status: Vec<String>,

    /// Only include tours of the given type; comma-separated (planned,recorded)
    #[arg(long = "type", value_delimiter = ',')]
    tour_type: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct ExportArgs {
    #[arg(long, default_value = "tours")]
    output_dir: PathBuf,

    #[command(flatten)]
    filters: FilterArgs,
}

#[derive(clap::Args, Debug)]
struct ListArgs {
    #[command(flatten)]
    filters: FilterArgs,
}

fn run_export(email: Option<String>, password: Option<String>, args: ExportArgs) -> Result<()> {
    let filters = filter::build_filters(
        args.filters.from_date.as_deref(),
        args.filters.to_date.as_deref(),
        &args.filters.status,
        &args.filters.tour_type,
    )?;
    let email = email.ok_or_else(|| anyhow!("KOMOOT_EMAIL or --email is required"))?;
    let password = match password {
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

fn run_list(email: Option<String>, password: Option<String>, args: ListArgs) -> Result<()> {
    let filters = filter::build_filters(
        args.filters.from_date.as_deref(),
        args.filters.to_date.as_deref(),
        &args.filters.status,
        &args.filters.tour_type,
    )?;
    let email = email.ok_or_else(|| anyhow!("KOMOOT_EMAIL or --email is required"))?;
    let password = match password {
        Some(pwd) => pwd,
        None => rpassword::prompt_password("Komoot password: ")
            .context("failed to read password from terminal")?,
    };

    let client = api::HttpKomootClient::authenticate(email, password)?;
    let markdown = list::list_with_client(&client, &filters)?;
    print!("{markdown}");
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let Cli {
        email,
        password,
        command,
    } = cli;
    let result = match command {
        Commands::Routes {
            command: RoutesCommands::Export(args),
        } => run_export(email, password, args),
        Commands::Routes {
            command: RoutesCommands::List(args),
        } => run_list(email, password, args),
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
