//! Following a hyperlink out of the document, or to somewhere inside it.
//!
//! A `<w:hyperlink>` says one of two things: `r:id`, naming a relationship
//! whose target is a URL the package never fetches, or `w:anchor`, naming a
//! bookmark in this document. The first leaves for the desktop's browser; the
//! second is a caret move and a scroll.
//!
//! **Why the scheme is checked.** Handing a URL to the shell means asking the
//! registry what opens that scheme, and a string that is not a URL at all —
//! a path to a program, a shortcut, a script — is something the registry will
//! open just as willingly. A document is not a trustworthy author: it arrived
//! from somewhere. So only the schemes that can be nothing but a thing to
//! fetch or address are followed, and every other target is shown to the user
//! to decide about instead of run. Word draws the same line, with a prompt.

use std::process::Command;

/// Where a link points, once its relationship has been looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Out of the document: a URL, exactly as the relationship states it.
    Away(String),
    /// A bookmark in this document.
    Here(String),
}

/// The schemes a link may be followed on without asking.
///
/// Each of these addresses a document or a correspondent and cannot name a
/// program. `file:` is deliberately absent: it is how a link runs an
/// executable, and a local path is worth showing before it is opened.
const FOLLOWED: [&str; 5] = ["http", "https", "mailto", "ftp", "ftps"];

/// Whether `url` is one this hands to the desktop unasked.
pub fn is_followed(url: &str) -> bool {
    // A control character is a way of hiding what the rest of the string is,
    // and no real target contains one.
    if url.is_empty() || url.chars().any(|c| c.is_control()) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    // A scheme is letters, digits and a little punctuation, and nothing else —
    // "https://x" split at the first colon gives one, "C" of "C:\x" gives one
    // too, which is why the list below is what decides and not the shape.
    FOLLOWED
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

/// Opens `url` with whatever the desktop uses for it.
///
/// Returns what went wrong, for the app to say. The launcher is asked and not
/// waited for: a browser takes seconds to start and the document must not stop
/// for it.
pub fn open(url: &str) -> Result<(), String> {
    if !is_followed(url) {
        return Err(format!("{url} is not a kind of link Scriva will open."));
    }
    launch(url).map_err(|error| format!("{url} could not be opened: {error}"))
}

#[cfg(windows)]
fn launch(url: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // The shell's own URL handler, rather than `cmd /c start`, which would
    // parse the target as a command line before opening anything.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn launch(url: &str) -> std::io::Result<()> {
    Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_address_is_followed_and_a_program_is_not() {
        assert!(is_followed("http://calibre-ebook.com/download"));
        assert!(is_followed("https://example.com/a?b=c#d"));
        assert!(is_followed("HTTPS://EXAMPLE.COM/"));
        assert!(is_followed("mailto:someone@example.com"));

        // Everything a document could say that is not a document to fetch.
        assert!(!is_followed("file:///C:/Windows/System32/cmd.exe"));
        assert!(!is_followed(r"C:\Windows\System32\cmd.exe"));
        assert!(!is_followed(r"\\server\share\payload.exe"));
        assert!(!is_followed("javascript:alert(1)"));
        assert!(!is_followed("ms-msdt:/id"));
        assert!(!is_followed("cmd.exe"));
        assert!(!is_followed(""));
        assert!(!is_followed("https:"));
        assert!(!is_followed("http://exa\nmple.com/"));
    }

    #[test]
    fn a_link_scriva_will_not_open_is_refused_rather_than_run() {
        let refused = open("file:///C:/Windows/System32/cmd.exe").expect_err("refused");
        assert!(
            refused.contains("not a kind of link"),
            "and says so: {refused}"
        );
    }
}
