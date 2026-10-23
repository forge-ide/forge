//! Forge CLI: a thin client of the daemon API.
//!
//! Results print to stdout as JSON; errors print to stderr with a non-zero exit status.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use forge_api::client::Client;
use forge_api::methods::*;
use forge_api::{API_VERSION, ProjectId, major_version};
use forge_config::Paths;
use serde::Serialize;

#[derive(Parser)]
#[command(version, about = "Forge CLI: a thin client of the forged daemon")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the daemon's version, platform, and sandbox tiers.
    Status,
    /// Open, list, and inspect projects.
    #[command(subcommand)]
    Project(ProjectCmd),
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Open the project at PATH, creating forge.toml if it has none.
    Open {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List known projects.
    List,
    /// Show one project.
    Get { project_id: ProjectId },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)
        .and_then(|rt| rt.block_on(run(cli.command)));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("forge: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Cmd) -> anyhow::Result<()> {
    let (mut client, hello) = connect().await?;
    match command {
        Cmd::Status => print(&hello),
        Cmd::Project(ProjectCmd::Open { path }) => {
            // The daemon has its own working directory, so send an absolute path.
            let path = path
                .canonicalize()
                .with_context(|| format!("resolving {}", path.display()))?;
            let path = path.to_str().context("path is not valid UTF-8")?.to_owned();
            print(
                &client
                    .call::<ProjectOpen>(&ProjectOpenParams { path })
                    .await?,
            )
        }
        Cmd::Project(ProjectCmd::List) => print(&client.call::<ProjectList>(&Empty {}).await?),
        Cmd::Project(ProjectCmd::Get { project_id }) => print(
            &client
                .call::<ProjectGet>(&ProjectGetParams { project_id })
                .await?,
        ),
    }
}

/// Connect and check that the daemon speaks the same major API version.
async fn connect() -> anyhow::Result<(Client, HelloResult)> {
    let socket = Paths::from_env()?.socket();
    let mut client = Client::connect(&socket)
        .await
        .context("cannot reach the daemon; is forged running? Start it with `forged`")?;
    let hello = client
        .call::<ServerHello>(&HelloParams {
            client: "forge-cli".into(),
            api_version: API_VERSION.into(),
        })
        .await?;
    if major_version(&hello.api_version) != major_version(API_VERSION) {
        bail!(
            "daemon API {} is incompatible with this CLI's API {API_VERSION}; restart forged from the same release",
            hello.api_version
        );
    }
    Ok((client, hello))
}

fn print(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
