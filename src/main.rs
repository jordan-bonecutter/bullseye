mod app;
mod capture;
mod clipboard;
mod document;

use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct Args {
    demo: bool,
    list_outputs: bool,
    open: Option<PathBuf>,
    output: Option<String>,
    cursor: bool,
    fullscreen: bool,
    delay: u64,
    save: Option<PathBuf>,
    save_to: Option<PathBuf>,
    copy: bool,
}
fn args() -> Result<Option<Args>> {
    let mut args = Args::default();
    let mut values = std::env::args().skip(1);
    while let Some(value) = values.next() {
        match value.as_str() {
            "-h" | "--help" => {
                println!(
                    "Bullseye — a good shot, a little polish.\n\nUsage: bullseye [OPTIONS]\n\n  --demo               Try the editor with a sample desktop\n  --open FILE          Annotate an existing PNG/JPEG\n  --output NAME        Capture a Wayland output (e.g. DP-1)\n  --list-outputs       List Wayland output connector names\n  --cursor             Include the pointer in the screenshot\n  --fullscreen         Open the editor fullscreen\n  --delay SECONDS      Wait before capture (0–60)\n  --save FILE          Capture directly to PNG without opening the editor\n  --save-to FILE       Choose the editor’s PNG save location\n  --copy               Copy directly to Wayland clipboard without opening the editor\n  -h, --help           Show this help\n\nWithout options, captures the first advertised output and opens the editor.\nCapture requires wlr-screencopy (Sway/wlroots); portal capture is not yet supported."
                );
                return Ok(None);
            }
            "--demo" => args.demo = true,
            "--list-outputs" => args.list_outputs = true,
            "--cursor" => args.cursor = true,
            "--fullscreen" => args.fullscreen = true,
            "--copy" => args.copy = true,
            "--open" => {
                args.open = Some(values.next().context("--open requires a filename")?.into())
            }
            "--output" => {
                args.output = Some(
                    values
                        .next()
                        .context("--output requires a connector name")?,
                )
            }
            "--save" => {
                args.save = Some(values.next().context("--save requires a filename")?.into())
            }
            "--save-to" => {
                args.save_to = Some(
                    values
                        .next()
                        .context("--save-to requires a filename")?
                        .into(),
                )
            }
            "--delay" => {
                args.delay = values
                    .next()
                    .context("--delay requires seconds")?
                    .parse()
                    .context("Delay must be a whole number")?;
                if args.delay > 60 {
                    bail!("Delay must be between 0 and 60 seconds");
                }
            }
            "--clipboard-server" => {
                clipboard::serve()?;
                return Ok(None);
            }
            _ => bail!("Unknown option: {value}. See --help"),
        }
    }
    if args.demo && args.open.is_some() {
        bail!("Choose either --demo or --open");
    }
    Ok(Some(args))
}
fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    home.join("Pictures")
        .join("Bullseye")
        .join(format!("bullseye-{seconds}.png"))
}
pub fn save_png(image: &image::RgbaImage, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).with_context(|| format!("Create {}", parent.display()))?;
    // Encode before replacing the destination so a failed export cannot truncate a previous shot.
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    image.write_to(&mut temp, image::ImageFormat::Png)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .with_context(|| format!("Save {}", path.display()))?;
    Ok(())
}
fn run() -> Result<()> {
    let Some(args) = args()? else {
        return Ok(());
    };
    if args.list_outputs {
        for name in capture::list_outputs()? {
            println!("{name}");
        }
        return Ok(());
    }
    if args.delay > 0 {
        std::thread::sleep(Duration::from_secs(args.delay));
    }
    let (image, label) = if args.demo {
        (document::demo_image(), "Demo workspace · 1440 × 900".into())
    } else if let Some(path) = &args.open {
        (
            image::open(path)
                .with_context(|| format!("Open {}", path.display()))?
                .to_rgba8(),
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        capture::screenshot(args.output.clone(), args.cursor)?
    };
    if let Some(path) = &args.save {
        save_png(&image, path)?;
        println!("Saved {}", path.display());
    }
    if args.copy {
        clipboard::copy(&image)?;
        println!("Copied screenshot to Wayland clipboard");
    }
    if args.save.is_some() || args.copy {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Bullseye — Screenshot editor")
            .with_app_id("bullseye")
            .with_inner_size([1280., 820.])
            .with_min_inner_size([1000., 680.])
            .with_fullscreen(args.fullscreen),
        ..Default::default()
    };
    let destination = args.save_to.unwrap_or_else(default_path);
    let recapture = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let request = recapture.clone();
    eframe::run_native(
        "Bullseye",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::Editor::new(
                cc,
                image,
                label,
                destination,
                request,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Open editor: {e}"))?;
    if recapture.load(std::sync::atomic::Ordering::Relaxed) {
        // Drop the old Wayland window before starting a new capture. Minimize is not
        // implemented consistently on tiling compositors, so it cannot exclude our UI.
        std::thread::sleep(Duration::from_millis(250));
        let mut command = std::process::Command::new(std::env::current_exe()?);
        if let Some(output) = args.output {
            command.arg("--output").arg(output);
        }
        if args.cursor {
            command.arg("--cursor");
        }
        if args.fullscreen {
            command.arg("--fullscreen");
        }
        command.spawn().context("Start a fresh capture")?;
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("bullseye: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_is_png_even_with_other_extension_and_replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/shot.data");
        let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]));
        save_png(&image, &path).unwrap();
        save_png(&image, &path).unwrap();
        assert_eq!(
            image::load_from_memory(&std::fs::read(path).unwrap())
                .unwrap()
                .to_rgba8(),
            image
        );
    }
}
