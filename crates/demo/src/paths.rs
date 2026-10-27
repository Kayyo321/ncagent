/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Where demo state lives (D40): a local data directory, labelled and separate from any future
//! production log.
//!
//! The lookup takes its environment as a closure, so the tests never read the real environment.

use std::path::{Path, PathBuf};

/// The platforms D40 names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

/// The demo state directory for a platform, from an environment lookup.
///
/// Linux: `$XDG_DATA_HOME/ncagent/demo` when that variable is set to an absolute path, otherwise
/// `$HOME/.local/share/ncagent/demo`. macOS: `$HOME/Library/Application Support/ncagent/demo`.
/// Windows: `%APPDATA%\ncagent\demo`. A missing variable is an error that names it.
pub fn state_dir_from(
    env: &dyn Fn(&str) -> Option<String>,
    platform: Platform,
) -> Result<PathBuf, String> {
    match platform {
        Platform::Linux => {
            if let Some(data) = env("XDG_DATA_HOME")
                .filter(|value| !value.is_empty() && Path::new(value).is_absolute())
            {
                return Ok(PathBuf::from(data).join("ncagent").join("demo"));
            }
            let home = required(env, "HOME")?;
            Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("ncagent")
                .join("demo"))
        }
        Platform::MacOs => {
            let home = required(env, "HOME")?;
            Ok(PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("ncagent")
                .join("demo"))
        }
        Platform::Windows => {
            let appdata = required(env, "APPDATA")?;
            Ok(PathBuf::from(appdata).join("ncagent").join("demo"))
        }
    }
}

/// The demo state directory for the platform this binary runs on.
pub fn default_state_dir() -> Result<PathBuf, String> {
    let platform = if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    };
    let lookup = |key: &str| std::env::var(key).ok();
    state_dir_from(&lookup, platform)
}

/// A variable that must be set and non-empty.
fn required(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Result<String, String> {
    match env(name) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(format!(
            "cannot place the demo state directory: {name} is not set"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |key: &str| {
            owned
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn linux_uses_xdg_data_home_when_it_is_absolute() {
        let env = env_of(&[("XDG_DATA_HOME", "/data/xdg"), ("HOME", "/home/u")]);
        let dir = state_dir_from(&env, Platform::Linux).expect("xdg wins");
        assert_eq!(dir, PathBuf::from("/data/xdg/ncagent/demo"));
    }

    #[test]
    fn linux_ignores_a_relative_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "relative/dir"), ("HOME", "/home/u")]);
        let dir = state_dir_from(&env, Platform::Linux).expect("falls back to HOME");
        assert_eq!(dir, PathBuf::from("/home/u/.local/share/ncagent/demo"));
    }

    #[test]
    fn linux_ignores_an_empty_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", ""), ("HOME", "/home/u")]);
        let dir = state_dir_from(&env, Platform::Linux).expect("falls back to HOME");
        assert_eq!(dir, PathBuf::from("/home/u/.local/share/ncagent/demo"));
    }

    #[test]
    fn linux_without_home_names_the_variable() {
        let env = env_of(&[]);
        let err = state_dir_from(&env, Platform::Linux).expect_err("HOME is required");
        assert!(err.contains("HOME"), "{err}");
    }

    #[test]
    fn macos_uses_application_support() {
        let env = env_of(&[("HOME", "/Users/u")]);
        let dir = state_dir_from(&env, Platform::MacOs).expect("macOS path");
        assert_eq!(
            dir,
            PathBuf::from("/Users/u")
                .join("Library")
                .join("Application Support")
                .join("ncagent")
                .join("demo")
        );
    }

    #[test]
    fn macos_without_home_names_the_variable() {
        let err = state_dir_from(&env_of(&[]), Platform::MacOs).expect_err("HOME is required");
        assert!(err.contains("HOME"), "{err}");
    }

    #[test]
    fn windows_uses_appdata_and_names_it_when_missing() {
        let env = env_of(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")]);
        let dir = state_dir_from(&env, Platform::Windows).expect("Windows path");
        assert_eq!(
            dir,
            PathBuf::from("C:\\Users\\u\\AppData\\Roaming")
                .join("ncagent")
                .join("demo")
        );
        let err = state_dir_from(&env_of(&[]), Platform::Windows).expect_err("APPDATA required");
        assert!(err.contains("APPDATA"), "{err}");
    }
}
