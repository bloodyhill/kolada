//! The desktop's own file chooser, through the XDG desktop portal
//! (`org.freedesktop.portal.FileChooser`): GNOME's on GNOME, KDE's on KDE,
//! and the one a sandbox (Flatpak, Snap) allows. Called from a thread: it
//! waits for the person. Where there is no portal, the shell falls back to
//! zenity (`outside.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// What is asked of the chooser.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Files (or one folder) to open.
    Open { title: String, multiple: bool, directory: bool },
    /// Where to save, starting with `name` in `folder`.
    Save { title: String, name: String, folder: PathBuf },
}

/// The portal could not be asked (none on this desktop).
#[derive(Debug)]
pub struct NoPortal;

const DESKTOP: &str = "org.freedesktop.portal.Desktop";

/// The request's object path the portal answers on, for our bus name and
/// `token` (asked for before the call, so the answer is never missed).
pub fn request_path(unique: &str, token: &str) -> String {
    format!("/org/freedesktop/portal/desktop/request/{}/{token}", unique.trim_start_matches(':').replace('.', "_"))
}

/// A folder as the portal takes it: its bytes, ending in a NUL.
pub fn folder_bytes(folder: &Path) -> Vec<u8> {
    let mut b = folder.as_os_str().as_encoded_bytes().to_vec();
    b.push(0);
    b
}

/// Asks the person; `parent` names the window it belongs to (`x11:1a00002`,
/// or empty). Ok(empty): cancelled.
pub fn choose(ask: &Ask, parent: &str) -> Result<Vec<PathBuf>, NoPortal> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let conn = Connection::session().map_err(|_| NoPortal)?;
    let unique = conn.unique_name().ok_or(NoPortal)?.to_string();
    let token = format!("kolada_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let path = request_path(&unique, &token);
    let request = Proxy::new(&conn, DESKTOP, path.as_str(), "org.freedesktop.portal.Request").map_err(|_| NoPortal)?;
    let mut answers = request.receive_signal("Response").map_err(|_| NoPortal)?;
    let chooser = Proxy::new(&conn, DESKTOP, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").map_err(|_| NoPortal)?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("modal", Value::from(true));
    let (method, title) = match ask {
        Ask::Open { title, multiple, directory } => {
            options.insert("multiple", Value::from(*multiple));
            options.insert("directory", Value::from(*directory));
            ("OpenFile", title)
        }
        Ask::Save { title, name, folder } => {
            options.insert("current_name", Value::from(name.as_str()));
            options.insert("current_folder", Value::from(folder_bytes(folder)));
            ("SaveFile", title)
        }
    };
    let _handle: OwnedObjectPath = chooser.call(method, &(parent, title.as_str(), options)).map_err(|_| NoPortal)?;
    let answer = answers.next().ok_or(NoPortal)?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer.body().deserialize().map_err(|_| NoPortal)?;
    if code != 0 {
        return Ok(Vec::new());
    }
    let uris: Vec<String> = results.get("uris").and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
    Ok(uris.iter().filter_map(|u| kolada_app::model::arrive::file_url_path(u)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_looked_for_where_the_portal_sends_it() {
        assert_eq!(request_path(":1.42", "kolada_7_1"), "/org/freedesktop/portal/desktop/request/1_42/kolada_7_1");
        assert_eq!(folder_bytes(Path::new("/home/a/Documents")), b"/home/a/Documents\0".to_vec());
    }
}
