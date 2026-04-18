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

    /// Session — scaffold only. F-024 wires real session routing and state.
    /// See `docs/architecture/window-hierarchy.md` §3.2.
    pub fn session(id: &str) -> Self {
        Self {
            label: format!("session-{id}"),
            title: format!("Forge \u{2014} Session {id}"),
            url: format!("/session/{id}"),
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
    fn session_spec_has_correct_dimensions() {
        let spec = WindowSpec::session("abc123");
        assert_eq!(spec.width, 1440.0);
        assert_eq!(spec.height, 900.0);
        assert_eq!(spec.min_width, 1024.0);
        assert_eq!(spec.min_height, 640.0);
    }

    #[test]
    fn session_spec_title_includes_id() {
        assert_eq!(
            WindowSpec::session("abc123").title,
            "Forge \u{2014} Session abc123"
        );
    }

    #[test]
    fn session_spec_label_includes_id() {
        assert_eq!(WindowSpec::session("abc123").label, "session-abc123");
    }

    #[test]
    fn session_spec_uses_session_route() {
        assert_eq!(WindowSpec::session("abc123").url, "/session/abc123");
    }

    #[test]
    fn session_spec_resizable_and_chrome() {
        let spec = WindowSpec::session("x");
        assert!(spec.resizable, "session must be resizable");
        assert!(spec.decorations, "session must use standard chrome");
        assert!(spec.center, "session must center on launch");
    }
}
