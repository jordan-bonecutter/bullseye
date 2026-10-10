//! Private D-Bus FileChooser fixture. Never registers a service on the desktop session.
use std::{collections::HashMap, path::PathBuf, time::Duration};
use zbus::{
    blocking::{Connection, connection::Builder},
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};
struct Chooser {
    control: PathBuf,
}
#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl Chooser {
    #[zbus(property)]
    fn version(&self) -> u32 {
        4
    }
    fn save_file(
        &self,
        _parent: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(title, "Save screenshot");
        assert!(options.contains_key("current_name"));
        assert!(options.contains_key("filters"));
        if std::fs::read_to_string(&self.control).unwrap().trim() == "error" {
            return Err(zbus::fdo::Error::Failed(
                "No FileChooser backend installed".into(),
            ));
        }
        let token = <&str>::try_from(options.get("handle_token").unwrap()).unwrap();
        let sender = header.sender().unwrap().to_string();
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{}/{}",
            sender.trim_start_matches(':').replace('.', "_"),
            token
        ))
        .unwrap();
        let conn = Connection::from(connection.clone());
        let signal_path = path.clone();
        let control = self.control.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let mode = std::fs::read_to_string(&control).unwrap();
            let mut results = HashMap::<String, OwnedValue>::new();
            let code = if mode.trim() == "cancel" {
                1_u32
            } else {
                let uri = format!("file://{}", mode.trim());
                results.insert(
                    "uris".into(),
                    Value::from(vec![uri]).try_to_owned().unwrap(),
                );
                0
            };
            conn.emit_signal(
                Some(sender.as_str()),
                signal_path,
                "org.freedesktop.portal.Request",
                "Response",
                &(code, results),
            )
            .unwrap();
        });
        Ok(path)
    }
}
fn main() -> anyhow::Result<()> {
    let runtime = std::env::var("XDG_RUNTIME_DIR")?;
    anyhow::ensure!(
        runtime.starts_with("/tmp/bullseye-headless-"),
        "Private headless runtime required"
    );
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")?;
    anyhow::ensure!(address.contains(&runtime), "Private D-Bus socket required");
    let control = PathBuf::from(std::env::args().nth(1).unwrap());
    let _conn = Builder::session()?
        .name("org.freedesktop.portal.Desktop")?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Chooser {
                control: control.clone(),
            },
        )?
        .build()?;
    std::fs::write(control.with_extension("ready"), "ready")?;
    loop {
        std::thread::park();
    }
}
