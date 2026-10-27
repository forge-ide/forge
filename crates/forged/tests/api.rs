//! API tests against a real daemon: project and subscription methods and their error paths.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{Daemon, expect_api_error, git_repo, temp_home};
use forge_api::client::ClientError;
use forge_api::events::Topic;
use forge_api::methods::*;
use forge_api::rpc::codes;
use forge_api::{API_VERSION, ErrorKind, ProjectId, SessionId, SubscriptionId};
use serde_json::json;

fn open(path: &std::path::Path) -> ProjectOpenParams {
    ProjectOpenParams {
        path: path.to_str().unwrap().to_owned(),
    }
}

#[tokio::test]
async fn hello_reports_versions_and_platform() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let hello = client
        .call::<ServerHello>(&HelloParams {
            client: "test".into(),
            api_version: API_VERSION.into(),
        })
        .await
        .unwrap();
    assert_eq!(hello.api_version, API_VERSION);
    assert_eq!(hello.daemon_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(hello.platform, forge_api::model::Platform::current());
    // Unsandboxed always runs (behind each profile's opt-in); the container tier doesn't yet.
    use forge_api::model::SandboxTier;
    assert!(hello.sandbox_tiers.contains(&SandboxTier::Unsandboxed));
    assert!(!hello.sandbox_tiers.contains(&SandboxTier::Container));
}

#[tokio::test]
async fn socket_is_private_to_the_user() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mode = std::fs::metadata(&daemon.socket)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let dir_mode = std::fs::metadata(home.path().join("run"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
}

#[tokio::test]
async fn open_writes_forge_toml_and_is_idempotent() {
    let home = temp_home();
    let repo = git_repo(home.path(), "demo");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;

    let project = client.call::<ProjectOpen>(&open(&repo)).await.unwrap();
    assert!(project.id.as_str().starts_with("prj_"));
    assert_eq!(project.name, "demo");
    assert_eq!(project.root, repo.to_str().unwrap());
    assert_eq!(project.repos.len(), 1);
    assert_eq!(project.repos[0].path, ".");
    assert_eq!(project.repos[0].base_branch, "main");

    let written = std::fs::read_to_string(repo.join("forge.toml")).unwrap();
    assert!(written.contains("base_branch = \"main\""), "{written}");

    let again = client.call::<ProjectOpen>(&open(&repo)).await.unwrap();
    assert_eq!(again, project);

    let list = client.call::<ProjectList>(&Empty {}).await.unwrap();
    assert_eq!(list, vec![project.clone()]);
    let got = client
        .call::<ProjectGet>(&ProjectGetParams {
            project_id: project.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(got, project);
}

#[tokio::test]
async fn open_uses_existing_forge_toml_and_picks_up_edits() {
    let home = temp_home();
    let repo = git_repo(home.path(), "demo");
    std::fs::write(
        repo.join("forge.toml"),
        "[project]\nname = \"Demo Project\"\n[[repo]]\nname = \"demo\"\npath = \".\"\nbase_branch = \"trunk\"\n",
    )
    .unwrap();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;

    let first = client.call::<ProjectOpen>(&open(&repo)).await.unwrap();
    assert_eq!(first.name, "Demo Project");
    assert_eq!(first.repos[0].base_branch, "trunk");

    std::fs::write(
        repo.join("forge.toml"),
        "[project]\nname = \"Renamed\"\n[[repo]]\nname = \"demo\"\npath = \".\"\nbase_branch = \"main\"\n",
    )
    .unwrap();
    let second = client.call::<ProjectOpen>(&open(&repo)).await.unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.name, "Renamed");
    assert_eq!(second.repos[0].base_branch, "main");
}

#[tokio::test]
async fn open_rejects_bad_paths_and_config() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;

    let relative = ProjectOpenParams {
        path: "demo".into(),
    };
    expect_api_error(
        client.call::<ProjectOpen>(&relative).await,
        ErrorKind::ValidationFailed,
    );

    let missing = home.path().join("missing");
    expect_api_error(
        client.call::<ProjectOpen>(&open(&missing)).await,
        ErrorKind::NotFound,
    );

    let plain = home.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let err = expect_api_error(
        client.call::<ProjectOpen>(&open(&plain)).await,
        ErrorKind::ValidationFailed,
    );
    assert!(err.detail().contains("git"), "{err}");
    assert!(!plain.join("forge.toml").exists());

    let repo = git_repo(home.path(), "demo");
    let sub = repo.join("src");
    std::fs::create_dir(&sub).unwrap();
    expect_api_error(
        client.call::<ProjectOpen>(&open(&sub)).await,
        ErrorKind::ValidationFailed,
    );
    assert!(!sub.join("forge.toml").exists());

    std::fs::write(
        repo.join("forge.toml"),
        "[project]\nname = \"x\"\ncolour = \"red\"\n[[repo]]\nname = \"x\"\npath = \".\"\nbase_branch = \"main\"\n",
    )
    .unwrap();
    let err = expect_api_error(
        client.call::<ProjectOpen>(&open(&repo)).await,
        ErrorKind::ValidationFailed,
    );
    assert!(err.detail().contains("colour"), "{err}");

    std::fs::write(
        repo.join("forge.toml"),
        "[project]\nname = \"x\"\n[[repo]]\nname = \"x\"\npath = \"nope\"\nbase_branch = \"main\"\n",
    )
    .unwrap();
    expect_api_error(
        client.call::<ProjectOpen>(&open(&repo)).await,
        ErrorKind::ValidationFailed,
    );

    assert!(
        client
            .call::<ProjectList>(&Empty {})
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn get_unknown_project_is_not_found() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let params = ProjectGetParams {
        project_id: ProjectId::new(),
    };
    expect_api_error(
        client.call::<ProjectGet>(&params).await,
        ErrorKind::NotFound,
    );

    let bad_id = client
        .call_raw("project.get", json!({ "projectId": "ses_nope" }))
        .await;
    expect_api_error(bad_id, ErrorKind::ValidationFailed);
}

#[tokio::test]
async fn projects_survive_a_daemon_restart() {
    let home = temp_home();
    let repo = git_repo(home.path(), "demo");
    let daemon = Daemon::start(home.path());
    let project = {
        let mut client = daemon.client().await;
        client.call::<ProjectOpen>(&open(&repo)).await.unwrap()
    };
    daemon.stop();

    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    assert_eq!(
        client.call::<ProjectList>(&Empty {}).await.unwrap(),
        vec![project]
    );
}

#[tokio::test]
async fn subscribe_validates_topics() {
    let home = temp_home();
    let repo = git_repo(home.path(), "demo");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = client.call::<ProjectOpen>(&open(&repo)).await.unwrap();

    let sub = client
        .call::<EventsSubscribe>(&EventsSubscribeParams {
            topics: vec![Topic::Board, Topic::Approvals, Topic::Project(project.id)],
            after_seq: Some(0),
        })
        .await
        .unwrap();
    client
        .call::<EventsUnsubscribe>(&EventsUnsubscribeParams {
            subscription_id: sub.subscription_id.clone(),
        })
        .await
        .unwrap();
    let twice = client
        .call::<EventsUnsubscribe>(&EventsUnsubscribeParams {
            subscription_id: sub.subscription_id,
        })
        .await;
    expect_api_error(twice, ErrorKind::NotFound);

    let none = EventsSubscribeParams {
        topics: vec![],
        after_seq: None,
    };
    expect_api_error(
        client.call::<EventsSubscribe>(&none).await,
        ErrorKind::ValidationFailed,
    );

    let unknown_project = EventsSubscribeParams {
        topics: vec![Topic::Project(ProjectId::new())],
        after_seq: None,
    };
    expect_api_error(
        client.call::<EventsSubscribe>(&unknown_project).await,
        ErrorKind::NotFound,
    );

    let bad_topic = client
        .call_raw("events.subscribe", json!({ "topics": ["boards"] }))
        .await;
    expect_api_error(bad_topic, ErrorKind::ValidationFailed);
}

#[tokio::test]
async fn connections_cannot_drop_each_others_subscriptions() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut a = daemon.client().await;
    let mut b = daemon.client().await;
    let sub = a
        .call::<EventsSubscribe>(&EventsSubscribeParams {
            topics: vec![Topic::Board],
            after_seq: None,
        })
        .await
        .unwrap();
    let params = EventsUnsubscribeParams {
        subscription_id: sub.subscription_id.clone(),
    };
    expect_api_error(
        b.call::<EventsUnsubscribe>(&params).await,
        ErrorKind::NotFound,
    );
    a.call::<EventsUnsubscribe>(&params).await.unwrap();

    let unknown = EventsUnsubscribeParams {
        subscription_id: SubscriptionId::new(),
    };
    expect_api_error(
        a.call::<EventsUnsubscribe>(&unknown).await,
        ErrorKind::NotFound,
    );
}

#[tokio::test]
async fn query_for_unknown_session_is_not_found() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let params = EventsQueryParams {
        session_id: SessionId::new(),
        kinds: None,
        text: None,
        before: None,
        limit: 50,
    };
    expect_api_error(
        client.call::<EventsQuery>(&params).await,
        ErrorKind::NotFound,
    );
}

#[tokio::test]
async fn unknown_and_unimplemented_methods_are_method_not_found() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    for method in ["nope.nothing", "lsp.request"] {
        match client.call_raw(method, json!({})).await {
            Err(ClientError::Rpc { code, .. }) => {
                assert_eq!(code, codes::METHOD_NOT_FOUND, "{method}")
            }
            other => panic!("{method}: expected method-not-found, got {other:?}"),
        }
    }
    // The connection stays usable after errors.
    client.call::<ProjectList>(&Empty {}).await.unwrap();
}
