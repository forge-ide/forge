//! Forge CLI: a thin client of the daemon API.
//!
//! Results print to stdout as JSON; errors print to stderr with a non-zero exit status.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use forge_api::client::Client;
use forge_api::events::{ApprovalDecision, EventPayload, Remember, Topic};
use forge_api::methods::*;
use forge_api::model::{Mode, SandboxTier, SessionState};
use forge_api::{API_VERSION, ApprovalId, ProjectId, SessionId, major_version};
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
    /// Agent profiles, with whether each is installed and signed in.
    #[command(subcommand)]
    Agent(AgentCmd),
    /// Start, prompt, watch, and close sessions.
    #[command(subcommand)]
    Session(SessionCmd),
    /// Pending approvals, and deciding them.
    #[command(subcommand)]
    Approval(ApprovalCmd),
}

#[derive(Subcommand)]
enum AgentCmd {
    /// List the built-in profiles, and a project's own when PROJECT_ID is given. Each says
    /// whether its agent is installed and signed in, and if not, how to fix that.
    List { project_id: Option<ProjectId> },
}

#[derive(Subcommand)]
enum SessionCmd {
    /// Start a session; it begins in provisioning, and a prompt starts its first turn.
    Create {
        #[arg(long)]
        project: ProjectId,
        /// Agent profile ID: `claude`, `codex`, `gemini`, or one in `.forge/agents/`.
        #[arg(long)]
        agent: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        prompt: String,
        /// ask, accept-edits, auto, or full-access.
        #[arg(long, default_value = "accept-edits", value_parser = parse_wire::<Mode>)]
        mode: Mode,
        /// light, standard, container, or unsandboxed; the profile's tier when omitted.
        #[arg(long, value_parser = parse_wire::<SandboxTier>)]
        sandbox: Option<SandboxTier>,
        #[arg(long)]
        branch: Option<String>,
    },
    Get {
        session_id: SessionId,
    },
    List {
        project_id: ProjectId,
    },
    /// Start a turn with TEXT.
    Prompt {
        session_id: SessionId,
        text: String,
    },
    Cancel {
        session_id: SessionId,
    },
    /// Close a session, keeping its worktree.
    Close {
        session_id: SessionId,
        #[arg(long, default_value = "abandoned", value_parser = parse_wire::<SessionOutcome>)]
        outcome: SessionOutcome,
    },
    /// Print the session's events as JSON lines, from the start, until it stops working
    /// (reaches ready, review, paused, done, or failed with no turn running).
    Watch {
        session_id: SessionId,
    },
}

#[derive(Subcommand)]
enum ApprovalCmd {
    /// List pending approvals, oldest first.
    List {
        #[arg(long)]
        project: Option<ProjectId>,
    },
    /// Allow or deny one.
    Decide {
        approval_id: ApprovalId,
        #[arg(value_parser = parse_wire::<ApprovalDecision>)]
        decision: ApprovalDecision,
        /// Remember for the session or the project.
        #[arg(long, value_parser = parse_wire::<Remember>)]
        remember: Option<Remember>,
        /// The reason the agent hears with a denial.
        #[arg(long)]
        note: Option<String>,
    },
}

/// A value by its wire name (`accept-edits`, `standard`, `allow`).
fn parse_wire<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| format!("{text:?} is not one of the accepted values"))
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
        Cmd::Agent(AgentCmd::List { project_id }) => print(
            &client
                .call::<AgentList>(&AgentListParams { project_id })
                .await?,
        ),
        Cmd::Session(cmd) => session(&mut client, cmd).await,
        Cmd::Approval(ApprovalCmd::List { project }) => print(
            &client
                .call::<ApprovalList>(&ApprovalListParams {
                    project_id: project,
                })
                .await?,
        ),
        Cmd::Approval(ApprovalCmd::Decide {
            approval_id,
            decision,
            remember,
            note,
        }) => print(
            &client
                .call::<ApprovalDecide>(&ApprovalDecideParams {
                    approval_id,
                    decision,
                    remember,
                    note,
                })
                .await?,
        ),
    }
}

async fn session(client: &mut Client, cmd: SessionCmd) -> anyhow::Result<()> {
    match cmd {
        SessionCmd::Create {
            project,
            agent,
            title,
            prompt,
            mode,
            sandbox,
            branch,
        } => print(
            &client
                .call::<SessionCreate>(&SessionCreateParams {
                    project_id: project,
                    agent_profile: agent,
                    title,
                    prompt,
                    mode,
                    sandbox,
                    branch,
                    on_collision: None,
                })
                .await?,
        ),
        SessionCmd::Get { session_id } => print(
            &client
                .call::<SessionGet>(&SessionIdParams { session_id })
                .await?,
        ),
        SessionCmd::List { project_id } => print(
            &client
                .call::<SessionList>(&SessionListParams {
                    project_id,
                    states: None,
                })
                .await?,
        ),
        SessionCmd::Prompt { session_id, text } => print(
            &client
                .call::<SessionPrompt>(&SessionPromptParams { session_id, text })
                .await?,
        ),
        SessionCmd::Cancel { session_id } => print(
            &client
                .call::<SessionCancel>(&SessionIdParams { session_id })
                .await?,
        ),
        SessionCmd::Close {
            session_id,
            outcome,
        } => print(
            &client
                .call::<SessionClose>(&SessionCloseParams {
                    session_id,
                    outcome,
                })
                .await?,
        ),
        SessionCmd::Watch { session_id } => watch(client, session_id).await,
    }
}

/// Print every event of the session until it settles somewhere a human acts next.
async fn watch(client: &mut Client, session_id: SessionId) -> anyhow::Result<()> {
    client
        .call::<EventsSubscribe>(&EventsSubscribeParams {
            topics: vec![Topic::Session(session_id.clone())],
            after_seq: Some(0),
        })
        .await?;
    let mut turn_running = false;
    loop {
        let note = client.next_event().await?;
        println!("{}", serde_json::to_string(&note.event)?);
        match &note.event.payload {
            EventPayload::TurnStarted(_) => turn_running = true,
            EventPayload::TurnEnded(_) => turn_running = false,
            EventPayload::SessionState(change)
                if !turn_running
                    && matches!(
                        change.to,
                        SessionState::Ready
                            | SessionState::Review
                            | SessionState::Paused
                            | SessionState::Done
                            | SessionState::Failed
                    ) =>
            {
                // Replayed history settles too; only stop once the session is there now.
                let now = client
                    .call::<SessionGet>(&SessionIdParams {
                        session_id: session_id.clone(),
                    })
                    .await?;
                if now.state == change.to {
                    return Ok(());
                }
            }
            _ => {}
        }
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
