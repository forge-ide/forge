//! Pure declarative window configuration.
//!
//! `WindowSpec` captures every dimension, chrome, and routing decision for a
//! Forge window without pulling in a live Tauri runtime. This lets the rules
//! in `docs/architecture/window-hierarchy.md` §3 be unit-tested on any host.

/// Declarative shape of a Forge window — size, chrome, and entry route.
#[derive(Debug, Clone)]
pub struct WindowSpec {
    /// Tauri window label. Must be unique per window.
    pub label: String,
    /// OS-level window title.
    pub title: String,
    /// Path on the embedded Solid app's router.
    pub url: String,
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub resizable: bool,
    /// Standard OS chrome (titlebar, close/min/max). `false` means frameless.
    pub decorations: bool,
    /// Center the window on the active display when opened.
    pub center: bool,
}

impl WindowSpec {
    /// Dashboard — the primary window opened on app launch.
    /// See `docs/architecture/window-hierarchy.md` §3.1.
    pub fn dashboard() -> Self {
        Self {
            label: "dashboard".to_string(),
            title: "Forge".to_string(),
            url: "/".to_string(),
            width: 1280.0,
            height: 800.0,
            min_width: 960.0,
            min_height: 640.0,
            resizable: true,
            decorations: true,
            center: true,
        }
    }

    /// Workspace window — one per registered workspace.
    ///
    /// Sessions belong to the workspace: the window's label is
    /// `workspace-<workspace_id>` (the stable `WorkspaceId` written to the
    /// workspaces registry), and the initial URL points at one of its
    /// sessions via `/session/<session_id>`. When the user switches between
    /// sessions in the ChatPane sidebar, the frontend router navigates within
    /// the same window — the window label does not change.
    ///
    /// See `docs/architecture/window-hierarchy.md` §3.2.
    pub fn workspace_session(workspace_id: &str, session_id: &str) -> Self {
        Self {
            label: format!("workspace-{workspace_id}"),
            title: format!("Forge \u{2014} Workspace {workspace_id}"),
            url: format!("/session/{session_id}"),
            width: 1440.0,
            height: 900.0,
            min_width: 1024.0,
            min_height: 640.0,
            resizable: true,
            decorations: true,
            center: true,
        }
    }
}

/// Build the canonical Tauri window label for a `workspace_id`. Lives at the
/// module top-level (not on `WindowSpec`) so the bridge / authz helpers can
/// build the expected label without standing up a full spec.
pub fn workspace_label(workspace_id: &str) -> String {
    format!("workspace-{workspace_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_spec_has_correct_dimensions() {
        let spec = WindowSpec::dashboard();
        assert_eq!(spec.width, 1280.0);
        assert_eq!(spec.height, 800.0);
        assert_eq!(spec.min_width, 960.0);
        assert_eq!(spec.min_height, 640.0);
    }

    #[test]
    fn dashboard_spec_has_correct_title() {
        assert_eq!(WindowSpec::dashboard().title, "Forge");
    }

    #[test]
    fn dashboard_spec_uses_root_url() {
        assert_eq!(WindowSpec::dashboard().url, "/");
    }

    #[test]
    fn dashboard_spec_label_is_dashboard() {
        assert_eq!(WindowSpec::dashboard().label, "dashboard");
    }

    #[test]
    fn dashboard_spec_resizable_and_chrome() {
        let spec = WindowSpec::dashboard();
        assert!(spec.resizable, "dashboard must be resizable");
        assert!(spec.decorations, "dashboard must use standard chrome");
        assert!(spec.center, "dashboard must center on launch");
    }

    #[test]
    fn workspace_session_spec_has_correct_dimensions() {
        let spec = WindowSpec::workspace_session("ws01", "abc123");
        assert_eq!(spec.width, 1440.0);
        assert_eq!(spec.height, 900.0);
        assert_eq!(spec.min_width, 1024.0);
        assert_eq!(spec.min_height, 640.0);
    }

    #[test]
    fn workspace_session_spec_title_includes_workspace_id() {
        assert_eq!(
            WindowSpec::workspace_session("ws01", "abc123").title,
            "Forge \u{2014} Workspace ws01"
        );
    }

    #[test]
    fn workspace_session_spec_label_keys_on_workspace_id() {
        let spec = WindowSpec::workspace_session("ws01", "abc123");
        assert_eq!(spec.label, "workspace-ws01");
        // Two sessions in the same workspace produce the same label —
        // the workspace window is shared.
        let spec2 = WindowSpec::workspace_session("ws01", "def456");
        assert_eq!(spec.label, spec2.label);
    }

    #[test]
    fn workspace_session_spec_uses_session_route() {
        assert_eq!(
            WindowSpec::workspace_session("ws01", "abc123").url,
            "/session/abc123"
        );
    }

    #[test]
    fn workspace_label_matches_workspace_session_spec() {
        let spec = WindowSpec::workspace_session("ws01", "abc123");
        assert_eq!(workspace_label("ws01"), spec.label);
    }

    #[test]
    fn workspace_session_spec_resizable_and_chrome() {
        let spec = WindowSpec::workspace_session("ws01", "x");
        assert!(spec.resizable, "workspace window must be resizable");
        assert!(spec.decorations, "workspace window must use standard chrome");
        assert!(spec.center, "workspace window must center on launch");
    }
}
