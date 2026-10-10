//! The Rust clipboard server outlives the editor, until another selection replaces it.
use anyhow::{Context, Result, bail};
use image::{ImageFormat, RgbaImage};
use std::{
    io::{BufRead, BufReader, Cursor, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use wl_clipboard_rs::copy::{MimeType, Options, Source};

pub fn serve() -> Result<()> {
    let mut png = Vec::new();
    std::io::stdin()
        .take(256 * 1024 * 1024)
        .read_to_end(&mut png)?;
    let mut options = Options::new();
    options.foreground(true);
    match options.prepare_copy(
        Source::Bytes(png.into_boxed_slice()),
        MimeType::Specific("image/png".into()),
    ) {
        Ok(prepared) => {
            println!("OK");
            std::io::stdout().flush()?;
            prepared.serve()?;
        }
        Err(error) => {
            println!("ERROR: {error}");
            std::io::stdout().flush()?;
            bail!(error);
        }
    }
    Ok(())
}
pub fn copy(image: &RgbaImage) -> Result<()> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--clipboard-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Start Bullseye clipboard server")?;
    child
        .stdin
        .take()
        .context("Missing clipboard pipe")?
        .write_all(png.get_ref())?;
    let stdout = child
        .stdout
        .take()
        .context("Missing clipboard acknowledgement")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(Duration::from_secs(5));
    match result {
        Ok(Ok(line)) if line.trim() == "OK" => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "Wayland clipboard unavailable: {}",
                match other {
                    Ok(Ok(line)) => line.trim().to_owned(),
                    Ok(Err(e)) => e.to_string(),
                    Err(e) => e.to_string(),
                }
            );
        }
    }
}
