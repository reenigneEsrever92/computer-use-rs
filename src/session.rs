//! Preflight checks for the graphical session.
//!
//! `desktop-use` needs a live graphical session. The portal it negotiates with
//! is reached over the session bus, and the compositor delivers frames to a
//! PipeWire instance rooted in the session's runtime directory. None of that
//! exists for a system service, so the server checks up front and names exactly
//! what is missing instead of failing later with an opaque D-Bus error.

use std::path::PathBuf;

use crate::error::Error;

/// The environment variables a Wayland session must provide.
pub const REQUIRED_ENV: [&str; 3] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "WAYLAND_DISPLAY",
];

/// Locations `xdg-desktop-portal` is installed in, ahead of `PATH`.
const PORTAL_LOCATIONS: [&str; 2] = [
    "/usr/libexec/xdg-desktop-portal",
    "/usr/lib/xdg-desktop-portal/xdg-desktop-portal",
];

/// Check that the session environment is present.
///
/// `get_env` is injected rather than read directly so that tests can exercise a
/// session-less environment without mutating the process environment.
pub fn check_env(get_env: impl Fn(&str) -> Option<String>) -> Result<(), Error> {
    for variable in REQUIRED_ENV {
        match get_env(variable) {
            Some(value) if !value.is_empty() => {}
            _ => return Err(Error::MissingSessionEnv { variable }),
        }
    }
    Ok(())
}

/// Check the ambient process environment.
pub fn check_current_env() -> Result<(), Error> {
    check_env(|name| std::env::var(name).ok())
}

/// Locate the `xdg-desktop-portal` binary, if it is installed.
///
/// This is a hint, not the authoritative check: what matters is whether the
/// portal answers on the session bus, which the capture backend determines when
/// it connects. A missing binary is worth warning about early, but it is not
/// proof that portals are unavailable — a sandbox can provide the same service
/// without the binary being on this filesystem.
pub fn find_portal() -> Option<PathBuf> {
    let installed = PORTAL_LOCATIONS.iter().map(PathBuf::from);
    if let Some(found) = installed.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("xdg-desktop-portal"))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// An environment lookup, as `check_env` takes one.
    type EnvLookup = Box<dyn Fn(&str) -> Option<String>>;

    fn env_of(pairs: &[(&str, &str)]) -> EnvLookup {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        Box::new(move |name: &str| map.get(name).cloned())
    }

    fn complete_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("WAYLAND_DISPLAY", "wayland-0"),
        ]
    }

    #[test]
    fn a_complete_environment_passes() {
        assert!(check_env(env_of(&complete_env())).is_ok());
    }

    #[test]
    fn every_required_variable_is_reported_by_name() {
        for missing in REQUIRED_ENV {
            let present: Vec<_> = complete_env()
                .into_iter()
                .filter(|(name, _)| *name != missing)
                .collect();

            let error = check_env(env_of(&present)).expect_err("expected a named error");
            let message = error.to_string();
            assert!(
                message.contains(missing),
                "error for a missing {missing} did not name it: {message}"
            );
            assert!(
                matches!(error, Error::MissingSessionEnv { variable } if variable == missing),
                "error for a missing {missing} was the wrong variant: {error:?}"
            );
        }
    }

    #[test]
    fn an_empty_variable_counts_as_missing() {
        let mut env = complete_env();
        env[2] = ("WAYLAND_DISPLAY", "");
        let error = check_env(env_of(&env)).expect_err("expected an error for an empty value");
        assert!(
            matches!(error, Error::MissingSessionEnv { variable } if variable == "WAYLAND_DISPLAY")
        );
    }
}
