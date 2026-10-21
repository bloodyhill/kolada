//! Reminders through the desktop's notifications: the freedesktop.org
//! notification service on the session bus (`org.freedesktop.Notifications`,
//! which GNOME, KDE, XFCE, Cinnamon and the notification daemons of the
//! others serve), for the model's `Notify`. Sent from a thread of its own:
//! the bus is never waited on by the window. Where nothing serves it,
//! nothing shows.

use std::collections::HashMap;

use kolada_app::services::Notify;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

pub struct LinuxNotify;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Shows `title` and `text` as Kolada's, with its icon, the desktop told
/// which app it comes from (its .desktop file). The notification's id.
pub fn send(conn: &Connection, title: &str, text: &str) -> zbus::Result<u32> {
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("kolada"));
    // Normal: it goes into the desktop's list and does not stay on top.
    hints.insert("urgency", Value::from(1u8));
    let actions: Vec<&str> = Vec::new();
    let reply = conn.call_method(Some(NAME), PATH, Some(NAME), "Notify", &("Kolada", 0u32, "kolada", title, text, actions, hints, -1i32))?;
    reply.body().deserialize::<u32>()
}

impl Notify for LinuxNotify {
    fn notify(&self, title: &str, text: &str) {
        let (title, text) = (title.to_string(), text.to_string());
        std::thread::spawn(move || {
            if let Ok(conn) = Connection::session() {
                let _ = send(&conn, &title, &text);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

    /// The desktop's notification service, as a test stands it in: it
    /// keeps what it was asked to show.
    struct Shown(Arc<Mutex<Vec<(String, String, String, String, String)>>>);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Shown {
        #[allow(clippy::too_many_arguments)]
        fn notify(&self, app: String, _replaces: u32, icon: String, summary: String, body: String, _actions: Vec<String>, hints: HashMap<String, zbus::zvariant::OwnedValue>, _timeout: i32) -> u32 {
            let entry = hints.get("desktop-entry").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
            let mut shown = self.0.lock().unwrap();
            shown.push((app, icon, summary, body, entry));
            shown.len() as u32
        }
    }

    #[test]
    fn a_reminder_goes_to_the_desktops_notifications_as_koladas() {
        // A session bus of the test's own (never the desktop's).
        let Ok(mut bus) = Command::new("dbus-daemon").args(["--session", "--nofork", "--print-address"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
            eprintln!("no dbus-daemon here: skipped");
            return;
        };
        let mut address = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(bus.stdout.take().unwrap()), &mut address).unwrap();
        let address = address.trim().to_string();
        let shown = Arc::new(Mutex::new(Vec::new()));
        let _server = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().name(NAME).unwrap().serve_at(PATH, Shown(shown.clone())).unwrap().build().unwrap();
        let app = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().build().unwrap();
        assert_eq!(send(&app, "Call the bank", "Today 16:00 \u{00B7} Plan").unwrap(), 1);
        assert_eq!(shown.lock().unwrap().as_slice(), [("Kolada".into(), "kolada".into(), "Call the bank".into(), "Today 16:00 \u{00B7} Plan".into(), "kolada".into())]);
        let _ = bus.kill();
        let _ = bus.wait();
    }
}
