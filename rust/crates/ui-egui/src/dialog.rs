//! The system's file and question dialogs, for the editor. The editor runs on desktop systems; the
//! web player builds this crate only for the game's own parts (its interfaces and gamepads), and in
//! a browser, which has no such dialogs, they answer as if cancelled.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

#[cfg(target_arch = "wasm32")]
pub(crate) use web::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::{Path, PathBuf};

    /// Takes the same settings as the desktop dialog and picks nothing.
    #[derive(Default)]
    pub(crate) struct FileDialog;

    impl FileDialog {
        pub(crate) fn new() -> Self {
            Self
        }
        pub(crate) fn set_title(self, _title: impl Into<String>) -> Self {
            self
        }
        pub(crate) fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
            self
        }
        pub(crate) fn set_directory(self, _folder: impl AsRef<Path>) -> Self {
            self
        }
        pub(crate) fn set_file_name(self, _name: impl Into<String>) -> Self {
            self
        }
        pub(crate) fn pick_file(self) -> Option<PathBuf> {
            None
        }
        pub(crate) fn pick_folder(self) -> Option<PathBuf> {
            None
        }
        pub(crate) fn save_file(self) -> Option<PathBuf> {
            None
        }
    }

    pub(crate) enum MessageButtons {
        YesNo,
    }

    #[derive(PartialEq, Eq)]
    pub(crate) enum MessageDialogResult {
        Yes,
        No,
    }

    /// Takes the same settings as the desktop dialog and answers No.
    pub(crate) struct MessageDialog;

    impl MessageDialog {
        pub(crate) fn new() -> Self {
            Self
        }
        pub(crate) fn set_title(self, _title: impl Into<String>) -> Self {
            self
        }
        pub(crate) fn set_description(self, _text: impl Into<String>) -> Self {
            self
        }
        pub(crate) fn set_buttons(self, _buttons: MessageButtons) -> Self {
            self
        }
        pub(crate) fn show(self) -> MessageDialogResult {
            MessageDialogResult::No
        }
    }
}
