//! Persistent user + workspace settings (F-151).
//!
//! Shape and lifecycle mirror [`crate::approvals`]:
//! - Two tiers: **workspace** (`<root>/.forge/settings.toml`) and **user**
//!   (`{config_dir}/forge/settings.toml`).
//! - Atomic writes via `<path>.tmp` + rename on the same filesystem.
//! - Missing files return defaults — first-run is the common case.
//!
//! Unlike `ApprovalConfig` (a flat `Vec<ApprovalEntry>`), settings are a
//! **structured object** with nested sections. This has two consequences the
//! IPC layer relies on:
//!
//! 1. **`#[serde(default)]` everywhere.** Every field, and every nested
//!    struct, carries `#[serde(default)]` so a settings file containing only
//!    `[notifications]` still deserializes — the missing `[windows]` section
//!    falls back to defaults. Without this, adding a new section in a later
//!    release would break every older settings.toml in the wild.
//!
//! 2. **Deep field-level merge for workspace-overrides-user.** Workspace does
//!    *not* wholesale replace user (that would delete user prefs the moment a
//!    repo gained a `.forge/settings.toml`). Instead [`load_merged_in`] parses
//!    each tier's file as `toml::Value`, overlays workspace keys onto user
//!    keys at every depth, then deserializes the merged tree into
//!    `AppSettings`. The net effect: `workspace.notifications.bg_agents = "os"`
//!    overrides only that single scalar, leaving `user.windows.session_mode`
//!    intact. See [`apply_setting_update`] + [`save_workspace_settings_raw`]
//!    for the mirror-image invariant on the write path — `set_setting` must
//!    mutate the raw TOML tree, not re-serialize `AppSettings`, or the
//!    merge's "absent-means-absent" semantic silently collapses.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config_file::{load_toml_or_default, save_raw_atomic, save_toml_atomic};
use crate::Result;

/// Notification delivery mode for background-agent events (F-138 consumer).
/// Serialized as snake_case string so TOML files read naturally
/// (`bg_agents = "toast"`) and ts-rs emits a string literal union.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum NotificationMode {
    #[default]
    Toast,
    Os,
    Both,
    Silent,
}

/// Session-window layout preference (consumer: `docs/ui-specs/shell.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum SessionMode {
    #[default]
    Single,
    Split,
}

/// `[notifications]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct NotificationsSettings {
    #[serde(default)]
    pub bg_agents: NotificationMode,
}

/// `[windows]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct WindowsSettings {
    #[serde(default)]
    pub session_mode: SessionMode,
}

/// Top-level settings shape persisted to `settings.toml`.
///
/// Intentionally does **not** carry `#[serde(deny_unknown_fields)]` — the
/// schema is open for extension. A future release may add new sections; older
/// builds reading a newer file must silently ignore unknown keys rather than
/// refuse to load. This is the opposite of `ApprovalConfig`, where strict
/// validation protects the approval trust boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct AppSettings {
    #[serde(default)]
    pub notifications: NotificationsSettings,
    #[serde(default)]
    pub windows: WindowsSettings,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Workspace-scoped settings path: `<root>/.forge/settings.toml`.
pub fn workspace_settings_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".forge").join("settings.toml")
}

/// User-scoped settings path under the platform config dir.
pub fn user_settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("forge").join("settings.toml"))
}

/// Test seam / caller-supplied variant of [`user_settings_path`].
pub fn user_settings_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("forge").join("settings.toml")
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Read the user-scoped settings. Returns defaults if the file is absent.
pub async fn load_user_settings() -> Result<AppSettings> {
    match user_settings_path() {
        Some(p) => load_from_path(&p).await,
        None => Ok(AppSettings::default()),
    }
}

/// Test-friendly variant that reads from `<dir>/forge/settings.toml`.
pub async fn load_user_settings_in(config_dir: &Path) -> Result<AppSettings> {
    load_from_path(&user_settings_path_in(config_dir)).await
}

/// Read the workspace-scoped settings from `<root>/.forge/settings.toml`.
pub async fn load_workspace_settings(workspace_root: &Path) -> Result<AppSettings> {
    load_from_path(&workspace_settings_path(workspace_root)).await
}

async fn load_from_path(path: &Path) -> Result<AppSettings> {
    load_toml_or_default(path).await
}

// ---------------------------------------------------------------------------
// Save (atomic .tmp + rename)
// ---------------------------------------------------------------------------

/// Atomically write the user-scoped settings.
pub async fn save_user_settings(settings: &AppSettings) -> Result<()> {
    let path = user_settings_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve user config directory"))?;
    save_to_path(&path, settings).await
}

/// Test-friendly variant of [`save_user_settings`].
pub async fn save_user_settings_in(config_dir: &Path, settings: &AppSettings) -> Result<()> {
    save_to_path(&user_settings_path_in(config_dir), settings).await
}

/// Atomically write the workspace-scoped settings.
pub async fn save_workspace_settings(workspace_root: &Path, settings: &AppSettings) -> Result<()> {
    save_to_path(&workspace_settings_path(workspace_root), settings).await
}

async fn save_to_path(path: &Path, settings: &AppSettings) -> Result<()> {
    save_toml_atomic(path, settings).await
}

/// Atomically write `body` to the workspace settings file. Exposed so the
/// IPC layer can persist the output of [`apply_setting_update`] without a
/// struct round-trip (see module docs on why that would clobber sibling
/// fields).
pub async fn save_workspace_settings_raw(workspace_root: &Path, body: &str) -> Result<()> {
    save_raw_atomic(&workspace_settings_path(workspace_root), body.as_bytes()).await
}

/// Atomically write `body` to `<config_dir>/forge/settings.toml`.
pub async fn save_user_settings_raw_in(config_dir: &Path, body: &str) -> Result<()> {
    save_raw_atomic(&user_settings_path_in(config_dir), body.as_bytes()).await
}

// ---------------------------------------------------------------------------
// Merge: workspace overrides user at field granularity
// ---------------------------------------------------------------------------
//
// Why "raw TOML tree" and not `AppSettings`-level struct merge:
//
// `#[serde(default)]` on every field means deserialization fills in defaults
// for any section the file does not declare. So `AppSettings` lost the
// distinction between "workspace explicitly set `windows.session_mode = single`"
// and "workspace did not mention windows at all" — both deserialize identically.
// A struct-level merge on these two identical-looking structs would happily
// clobber a user's non-default `windows.session_mode` with the workspace's
// implicit default.
//
// The merge must therefore run on the raw `toml::Value` trees parsed from the
// respective files (or on an empty tree when the file is absent). Only keys
// the workspace file physically contains overlay user keys; missing sections
// fall through unchanged.

/// Load both tiers and return the merged result. Workspace keys override user
/// keys at every depth; sections and fields the workspace file does not
/// declare are preserved from user. Missing files are treated as empty trees.
///
/// This is the entry point the Tauri `get_settings` command uses — it
/// captures the merge semantic in one place so callers cannot accidentally
/// apply struct-level overrides (see module doc for why that would be wrong).
pub async fn load_merged_in(
    user_config_dir: Option<&Path>,
    workspace_root: &Path,
) -> Result<AppSettings> {
    let user_tree = match user_config_dir {
        Some(dir) => load_tree(&user_settings_path_in(dir)).await?,
        None => toml::Value::Table(toml::value::Table::new()),
    };
    let workspace_tree = load_tree(&workspace_settings_path(workspace_root)).await?;
    Ok(merge_trees_into_settings(user_tree, workspace_tree))
}

async fn load_tree(path: &Path) -> Result<toml::Value> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            let val: toml::Value = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
            Ok(val)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::value::Table::new()))
        }
        Err(e) => Err(e.into()),
    }
}

fn merge_trees_into_settings(mut user: toml::Value, workspace: toml::Value) -> AppSettings {
    merge_value(&mut user, workspace);
    user.try_into().unwrap_or_else(|_| AppSettings::default())
}

/// Recursively overlay `src` onto `dst`. Tables merge key-by-key; everything
/// else is overwritten wholesale. Arrays are overwritten (not concatenated) —
/// the only arrays the current schema could carry would be replacement lists,
/// not additive ones, so "workspace replaces" is the intuitive default.
fn merge_value(dst: &mut toml::Value, src: toml::Value) {
    match (dst, src) {
        (toml::Value::Table(dst_tbl), toml::Value::Table(src_tbl)) => {
            for (k, v) in src_tbl {
                match dst_tbl.get_mut(&k) {
                    Some(existing) => merge_value(existing, v),
                    None => {
                        dst_tbl.insert(k, v);
                    }
                }
            }
        }
        (dst, src) => *dst = src,
    }
}

// ---------------------------------------------------------------------------
// Apply a (key, value) update to a settings tier, preserving sibling fields.
// ---------------------------------------------------------------------------
//
// `set_setting` at the IPC layer MUST load → mutate the raw toml tree →
// validate (by deserializing to `AppSettings`) → save. If it instead
// struct-mutated `AppSettings` and re-saved, every sibling field would be
// re-serialized from its in-memory value — including fields that were absent
// from the file (and therefore default-filled on load). That would silently
// promote defaults into the persisted file and erase the
// absent-means-pick-up-default semantic the merge layer depends on.

/// Apply `(dotted_key, value)` to the raw TOML contents of a settings file,
/// returning the updated TOML text. Creates missing parent tables. Does not
/// touch the filesystem; the caller handles atomic-write.
///
/// The value is validated by deserializing the updated tree back into
/// [`AppSettings`] — unknown keys are allowed (forward-compat), but a type
/// mismatch (e.g. `bg_agents = 42`) is rejected with a structured error.
pub fn apply_setting_update(
    existing_toml: &str,
    dotted_key: &str,
    value: toml::Value,
) -> Result<String> {
    let mut tree: toml::Value = if existing_toml.trim().is_empty() {
        toml::Value::Table(toml::value::Table::new())
    } else {
        toml::from_str(existing_toml).map_err(|e| anyhow::anyhow!(e))?
    };

    let segments: Vec<&str> = dotted_key.split('.').collect();
    if segments.is_empty() || segments.iter().any(|s| s.is_empty()) {
        return Err(anyhow::anyhow!("invalid setting key: {dotted_key:?}").into());
    }

    // Navigate / create the nested tables, then set the leaf.
    {
        let mut cursor = tree
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("settings root must be a table"))?;
        for seg in &segments[..segments.len() - 1] {
            let entry = cursor
                .entry(seg.to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if !entry.is_table() {
                // A scalar is in the way (e.g. someone hand-edited a section
                // name to a string). Refuse rather than silently clobber it.
                return Err(
                    anyhow::anyhow!("cannot set {dotted_key}: {seg} is not a table").into(),
                );
            }
            cursor = entry.as_table_mut().expect("just asserted table");
        }
        let leaf = segments[segments.len() - 1].to_string();
        cursor.insert(leaf, value);
    }

    // Validate by deserializing into AppSettings. This rejects type errors at
    // the IPC boundary rather than letting a malformed file land on disk.
    let _validated: AppSettings = tree
        .clone()
        .try_into()
        .map_err(|e| anyhow::anyhow!("invalid setting value: {e}"))?;

    toml::to_string(&tree)
        .map_err(|e| anyhow::anyhow!(e))
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // TOML round-trip + #[serde(default)] behavior
    // -----------------------------------------------------------------------

    #[test]
    fn toml_roundtrip_preserves_all_fields() {
        let cfg = AppSettings {
            notifications: NotificationsSettings {
                bg_agents: NotificationMode::Both,
            },
            windows: WindowsSettings {
                session_mode: SessionMode::Split,
            },
        };
        let body = toml::to_string(&cfg).unwrap();
        let decoded: AppSettings = toml::from_str(&body).unwrap();
        assert_eq!(cfg, decoded);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Empty file → all defaults.
        let empty: AppSettings = toml::from_str("").unwrap();
        assert_eq!(empty, AppSettings::default());

        // Partial file: only notifications section set.
        let partial: AppSettings = toml::from_str(
            r#"
[notifications]
bg_agents = "os"
"#,
        )
        .unwrap();
        assert_eq!(partial.notifications.bg_agents, NotificationMode::Os);
        assert_eq!(partial.windows.session_mode, SessionMode::Single);

        // Partial file: only a sub-field inside a section set.
        let partial: AppSettings = toml::from_str(
            r#"
[windows]
session_mode = "split"
"#,
        )
        .unwrap();
        assert_eq!(partial.windows.session_mode, SessionMode::Split);
        assert_eq!(partial.notifications.bg_agents, NotificationMode::default());
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compat() {
        // Schema is open; a newer release's keys must not break older readers.
        let body = r#"
[notifications]
bg_agents = "toast"
future_field = "whatever"

[some_new_section]
x = 1
"#;
        let parsed: AppSettings = toml::from_str(body).unwrap();
        assert_eq!(parsed.notifications.bg_agents, NotificationMode::Toast);
    }

    #[test]
    fn notification_mode_serializes_to_snake_case() {
        let body = toml::to_string(&NotificationsSettings {
            bg_agents: NotificationMode::Os,
        })
        .unwrap();
        assert!(
            body.contains(r#"bg_agents = "os""#),
            "expected snake_case literal, got {body:?}"
        );
    }

    #[test]
    fn session_mode_serializes_to_snake_case() {
        let body = toml::to_string(&WindowsSettings {
            session_mode: SessionMode::Split,
        })
        .unwrap();
        assert!(
            body.contains(r#"session_mode = "split""#),
            "expected snake_case literal, got {body:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Defaults
    // -----------------------------------------------------------------------

    #[test]
    fn default_app_settings_matches_spec() {
        let def = AppSettings::default();
        assert_eq!(def.notifications.bg_agents, NotificationMode::Toast);
        assert_eq!(def.windows.session_mode, SessionMode::Single);
    }

    // -----------------------------------------------------------------------
    // Load
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn load_workspace_missing_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let cfg = load_workspace_settings(dir.path()).await.unwrap();
        assert_eq!(cfg, AppSettings::default());
    }

    #[tokio::test]
    async fn load_user_missing_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let cfg = load_user_settings_in(dir.path()).await.unwrap();
        assert_eq!(cfg, AppSettings::default());
    }

    // -----------------------------------------------------------------------
    // Save + round-trip
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn save_then_load_workspace_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cfg = AppSettings {
            notifications: NotificationsSettings {
                bg_agents: NotificationMode::Both,
            },
            windows: WindowsSettings {
                session_mode: SessionMode::Split,
            },
        };
        save_workspace_settings(dir.path(), &cfg).await.unwrap();
        let loaded = load_workspace_settings(dir.path()).await.unwrap();
        assert_eq!(loaded, cfg);
    }

    #[tokio::test]
    async fn save_then_load_user_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cfg = AppSettings {
            notifications: NotificationsSettings {
                bg_agents: NotificationMode::Silent,
            },
            windows: WindowsSettings::default(),
        };
        save_user_settings_in(dir.path(), &cfg).await.unwrap();
        let loaded = load_user_settings_in(dir.path()).await.unwrap();
        assert_eq!(loaded, cfg);
    }

    #[tokio::test]
    async fn save_workspace_creates_dot_forge_dir() {
        let dir = TempDir::new().unwrap();
        assert!(!dir.path().join(".forge").exists());
        save_workspace_settings(dir.path(), &AppSettings::default())
            .await
            .unwrap();
        assert!(dir.path().join(".forge").join("settings.toml").exists());
    }

    #[tokio::test]
    async fn save_is_atomic_via_tmp_and_rename() {
        let dir = TempDir::new().unwrap();
        let cfg = AppSettings {
            notifications: NotificationsSettings {
                bg_agents: NotificationMode::Os,
            },
            windows: WindowsSettings::default(),
        };
        save_workspace_settings(dir.path(), &cfg).await.unwrap();

        let final_path = dir.path().join(".forge").join("settings.toml");
        let tmp_path = dir.path().join(".forge").join("settings.toml.tmp");
        assert!(final_path.exists());
        assert!(!tmp_path.exists(), "tmp must be renamed, not left behind");

        // Overwrite + re-verify residue.
        save_workspace_settings(dir.path(), &AppSettings::default())
            .await
            .unwrap();
        assert!(final_path.exists());
        assert!(!tmp_path.exists());
    }

    // -----------------------------------------------------------------------
    // load_merged_in: workspace overrides user field-by-field
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn workspace_field_overrides_user_field_leaving_others_intact() {
        // User sets both fields to non-default values.
        let user_dir = TempDir::new().unwrap();
        save_user_settings_in(
            user_dir.path(),
            &AppSettings {
                notifications: NotificationsSettings {
                    bg_agents: NotificationMode::Toast,
                },
                windows: WindowsSettings {
                    session_mode: SessionMode::Split,
                },
            },
        )
        .await
        .unwrap();

        // Workspace overrides ONLY notifications.bg_agents — no [windows]
        // section at all. Write raw TOML so the file physically lacks the
        // windows table, not merely has it set to defaults.
        let workspace_dir = TempDir::new().unwrap();
        tokio::fs::create_dir_all(workspace_dir.path().join(".forge"))
            .await
            .unwrap();
        tokio::fs::write(
            workspace_dir.path().join(".forge").join("settings.toml"),
            "[notifications]\nbg_agents = \"os\"\n",
        )
        .await
        .unwrap();

        let merged = load_merged_in(Some(user_dir.path()), workspace_dir.path())
            .await
            .unwrap();
        assert_eq!(
            merged.notifications.bg_agents,
            NotificationMode::Os,
            "workspace overrides user on declared field"
        );
        assert_eq!(
            merged.windows.session_mode,
            SessionMode::Split,
            "user field survives when workspace file does not declare it"
        );
    }

    #[tokio::test]
    async fn merge_empty_workspace_returns_user_settings() {
        let user_dir = TempDir::new().unwrap();
        let user_cfg = AppSettings {
            notifications: NotificationsSettings {
                bg_agents: NotificationMode::Silent,
            },
            windows: WindowsSettings {
                session_mode: SessionMode::Split,
            },
        };
        save_user_settings_in(user_dir.path(), &user_cfg)
            .await
            .unwrap();

        // No workspace file on disk.
        let workspace_dir = TempDir::new().unwrap();
        let merged = load_merged_in(Some(user_dir.path()), workspace_dir.path())
            .await
            .unwrap();
        assert_eq!(merged, user_cfg);
    }

    #[tokio::test]
    async fn merge_no_user_dir_and_no_workspace_file_returns_defaults() {
        let workspace_dir = TempDir::new().unwrap();
        let merged = load_merged_in(None, workspace_dir.path()).await.unwrap();
        assert_eq!(merged, AppSettings::default());
    }

    // -----------------------------------------------------------------------
    // apply_setting_update: in-place field updates preserve siblings
    // -----------------------------------------------------------------------

    #[test]
    fn apply_setting_update_preserves_sibling_fields() {
        // Pre-existing file carries BOTH settings at non-default values. We
        // flip only `windows.session_mode` and assert `notifications.bg_agents`
        // survives unchanged in the output TOML text.
        let initial = r#"
[notifications]
bg_agents = "silent"

[windows]
session_mode = "single"
"#;
        let updated = apply_setting_update(
            initial,
            "windows.session_mode",
            toml::Value::String("split".into()),
        )
        .unwrap();
        let reparsed: AppSettings = toml::from_str(&updated).unwrap();
        assert_eq!(reparsed.notifications.bg_agents, NotificationMode::Silent);
        assert_eq!(reparsed.windows.session_mode, SessionMode::Split);
    }

    #[test]
    fn apply_setting_update_creates_missing_parent_tables() {
        let updated = apply_setting_update(
            "",
            "notifications.bg_agents",
            toml::Value::String("both".into()),
        )
        .unwrap();
        let reparsed: AppSettings = toml::from_str(&updated).unwrap();
        assert_eq!(reparsed.notifications.bg_agents, NotificationMode::Both);
    }

    #[test]
    fn apply_setting_update_rejects_type_mismatch() {
        // bg_agents is an enum string; an integer must fail validation.
        let err = apply_setting_update("", "notifications.bg_agents", toml::Value::Integer(42))
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("invalid setting value"),
            "expected validation failure, got {msg}"
        );
    }

    #[test]
    fn apply_setting_update_rejects_empty_segment() {
        let err = apply_setting_update("", "", toml::Value::String("x".into())).unwrap_err();
        assert!(format!("{err}").contains("invalid setting key"));

        let err = apply_setting_update(
            "",
            "notifications..bg_agents",
            toml::Value::String("toast".into()),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("invalid setting key"));
    }

    #[test]
    fn apply_setting_update_then_parse_matches_expected_scalar() {
        // Successive updates compose: set field A, then set field B, then set
        // field A again — end state reflects every write.
        let mut s = String::new();
        s = apply_setting_update(
            &s,
            "notifications.bg_agents",
            toml::Value::String("os".into()),
        )
        .unwrap();
        s = apply_setting_update(
            &s,
            "windows.session_mode",
            toml::Value::String("split".into()),
        )
        .unwrap();
        s = apply_setting_update(
            &s,
            "notifications.bg_agents",
            toml::Value::String("both".into()),
        )
        .unwrap();

        let reparsed: AppSettings = toml::from_str(&s).unwrap();
        assert_eq!(reparsed.notifications.bg_agents, NotificationMode::Both);
        assert_eq!(reparsed.windows.session_mode, SessionMode::Split);
    }

    // -----------------------------------------------------------------------
    // Paths
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_settings_path_is_under_dot_forge() {
        let p = workspace_settings_path(Path::new("/repo"));
        assert_eq!(p, Path::new("/repo/.forge/settings.toml"));
    }

    #[test]
    fn user_settings_path_in_nests_under_forge() {
        let p = user_settings_path_in(Path::new("/xdg"));
        assert_eq!(p, Path::new("/xdg/forge/settings.toml"));
    }
}
