//! The files that have been opened, most recent first.
//!
//! Kept next to the window geometry in `~/.config/<app>/recent`, one path per
//! line: a list a person can read, edit in a text editor, or delete outright
//! when they would rather the machine forgot.

use std::path::{Path, PathBuf};

use crate::AppId;

/// How many are kept.
///
/// A recent-files menu is a list to glance down, not one to search. Ten is
/// about as many as anyone recognises at a glance, and the eleventh file is
/// what the Open dialog is for.
const KEEP: usize = 10;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recent {
    paths: Vec<PathBuf>,
}

impl Recent {
    /// Reads the list, treating anything unreadable as an empty one.
    ///
    /// A missing file is the ordinary case on a first run, and a damaged one is
    /// not worth a complaint on startup about a convenience.
    pub fn load(app: AppId) -> Self {
        let Some(path) = Self::file(app) else {
            return Self::default();
        };
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::default(),
        }
    }

    /// Most recent first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Puts `path` at the front, and writes the list down.
    ///
    /// Written on every change rather than at exit, because the list is worth
    /// most after the crash that stopped you saving.
    pub fn remember(&mut self, app: AppId, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        // A path containing a newline cannot survive a line-per-path file, and
        // silently keeping half of it would name a file nobody has.
        if path.to_string_lossy().contains(['\n', '\r']) {
            return;
        }
        self.paths.retain(|kept| !same(kept, &path));
        self.paths.insert(0, path);
        self.paths.truncate(KEEP);
        self.store(app);
    }

    /// Drops `path` — for the entry that turns out to name a file that has been
    /// moved or deleted, which is no use to anybody as a menu item.
    pub fn forget(&mut self, app: AppId, path: &Path) {
        let before = self.paths.len();
        self.paths.retain(|kept| !same(kept, path));
        if self.paths.len() != before {
            self.store(app);
        }
    }

    pub fn clear(&mut self, app: AppId) {
        self.paths.clear();
        self.store(app);
    }

    /// The directory to start a file dialog in when the document has no path of
    /// its own: where the last file came from, which is where the next one
    /// probably is.
    pub fn directory(&self) -> Option<&Path> {
        self.paths.first()?.parent()
    }

    fn file(app: AppId) -> Option<PathBuf> {
        crate::paths::config_dir_path(app)
            .ok()
            .map(|dir| dir.join("recent"))
    }

    /// Best-effort, and silent about it, for the same reason the window
    /// geometry is: a list that will not be remembered is a small annoyance,
    /// and a dialog about it is a larger one.
    fn store(&self, app: AppId) {
        let Some(path) = Self::file(app) else { return };
        if crate::paths::config_dir(app).is_err() {
            return;
        }
        let _ = std::fs::write(path, self.text());
    }

    fn text(&self) -> String {
        self.paths
            .iter()
            .map(|p| format!("{}\n", p.display()))
            .collect()
    }

    fn parse(text: &str) -> Self {
        let mut recent = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let path = PathBuf::from(line);
            if recent.paths.iter().any(|kept| same(kept, &path)) {
                continue;
            }
            recent.paths.push(path);
            if recent.paths.len() == KEEP {
                break;
            }
        }
        recent
    }
}

/// Whether two paths name the same file, as far as the platform is concerned.
///
/// Windows paths are case-insensitive, and a list holding both `Book.xlsx` and
/// `book.xlsx` for the one file is a list that has failed at its only job.
fn same(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(lines: &[&str]) -> Recent {
        Recent {
            paths: lines.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn a_list_is_written_down_and_read_back_the_same() {
        let list = of(&["/home/a/one.xlsx", "/home/a/two.xlsx"]);
        assert_eq!(Recent::parse(&list.text()), list);
    }

    #[test]
    fn the_file_just_opened_comes_first() {
        let mut list = of(&["/a/one.xlsx", "/a/two.xlsx"]);
        list.paths.retain(|p| !same(p, Path::new("/a/two.xlsx")));
        list.paths.insert(0, PathBuf::from("/a/two.xlsx"));
        assert_eq!(list.paths()[0], Path::new("/a/two.xlsx"));
        assert_eq!(list.paths().len(), 2, "reopening a file should not add one");
    }

    #[test]
    fn a_list_longer_than_the_menu_is_cut_to_the_menu() {
        let lines: Vec<String> = (0..40).map(|i| format!("/a/{i}.xlsx")).collect();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        assert_eq!(Recent::parse(&text).paths().len(), KEEP);
    }

    #[test]
    fn blank_lines_and_stray_whitespace_are_not_files() {
        let list = Recent::parse("\n  /a/one.xlsx  \n\n\n/a/two.xlsx\n");
        assert_eq!(
            list.paths(),
            [PathBuf::from("/a/one.xlsx"), PathBuf::from("/a/two.xlsx")]
        );
    }

    #[test]
    fn the_same_file_is_not_listed_twice() {
        let list = Recent::parse("/a/one.xlsx\n/a/one.xlsx\n/a/two.xlsx\n");
        assert_eq!(list.paths().len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn windows_spells_one_file_several_ways_and_they_are_still_one_file() {
        let list = Recent::parse("C:\\A\\Book.xlsx\nc:\\a\\book.XLSX\n");
        assert_eq!(list.paths().len(), 1);
    }

    #[test]
    fn the_dialog_opens_where_the_last_file_came_from() {
        let list = of(&["/home/a/models/one.xlsx", "/home/a/two.xlsx"]);
        assert_eq!(list.directory(), Some(Path::new("/home/a/models")));
    }
}
