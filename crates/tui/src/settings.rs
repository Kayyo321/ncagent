/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The user's settings in the app (PROJECT.md, D63). The runtime keeps and checks them; this
//! module reads them once at start and saves each change through the runtime.
//!
//! A settings file that does not load leaves the defaults in use and is not overwritten by
//! anything the app saves on its own, such as the open tabs (D70). A change the user makes on
//! purpose (`update_settings`) still saves.

use harness::settings::settings_t;

use crate::app::{ToastKind, app_t};

impl app_t {
    /// Read the settings once at start. On an error one toast names it and the defaults are
    /// used (D70).
    pub(crate) fn load_settings(&mut self) {
        match self.runtime.settings() {
            Ok(settings) => {
                self.settings = settings;
                self.settings_ok = true;
            }
            Err(err) => {
                self.settings = settings_t::default();
                self.settings_ok = false;
                self.say(
                    format!("could not read settings, using the defaults: {err}"),
                    ToastKind::Warning,
                );
            }
        }
    }

    /// Change the settings with `change` and save them through the runtime. Nothing changes
    /// when the change is a no-op. A refused save is shown as a toast and leaves the settings as
    /// they were. Returns true when the settings now hold the change.
    pub fn update_settings(&mut self, change: impl FnOnce(&mut settings_t)) -> bool {
        let mut next = self.settings.clone();
        change(&mut next);
        if next == self.settings {
            return true;
        }
        match self.runtime.set_settings(next) {
            Ok(saved) => {
                self.settings = saved;
                self.settings_ok = true;
                true
            }
            Err(err) => {
                self.say(format!("could not save settings: {err}"), ToastKind::Error);
                false
            }
        }
    }

    /// Save a change the app makes on its own, such as the tab list. Skipped while the settings
    /// file did not load, so that file stays on disk as it is (D70).
    pub(crate) fn update_settings_quietly(&mut self, change: impl FnOnce(&mut settings_t)) {
        if self.settings_ok {
            self.update_settings(change);
        }
    }
}
