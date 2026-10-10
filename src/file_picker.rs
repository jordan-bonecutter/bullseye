//! Use the user's desktop portal backend; do not launch an external toolkit.
use anyhow::{Context, Result};
use ashpd::desktop::{
    ResponseError,
    file_chooser::{FileFilter, SaveFileRequest},
};
use std::path::{Path, PathBuf};
pub fn save(path: &Path) -> Result<Option<PathBuf>> {
    pollster::block_on(async {
        let folder = path
            .parent()
            .and_then(|p| p.ancestors().find(|p| p.is_dir()))
            .unwrap_or(Path::new("/tmp"));
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let request = SaveFileRequest::default()
            .title("Save screenshot")
            .current_name(name.as_ref())
            .current_folder(folder)?
            .filters([FileFilter::new("PNG image").glob("*.png")])
            .send()
            .await?;
        match request.response() {
            Ok(response) => {
                let uri = response
                    .uris()
                    .first()
                    .context("File picker returned no destination")?;
                Ok(Some(uri.to_file_path().map_err(|_| {
                    anyhow::anyhow!("Choose a local file destination")
                })?))
            }
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
}
