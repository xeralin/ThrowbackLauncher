use std::path::PathBuf;

use backend::log;
use gpui::{App, PathPromptOptions, SharedString};

pub(super) type Picked = Result<Option<PathBuf>, SharedString>;

fn pick(folder: bool, prompt: Option<&str>, cx: &App) -> impl Future<Output = Picked> + use<> {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: !folder,
        directories: folder,
        multiple: false,
        prompt: prompt.map(SharedString::new),
    });
    async move {
        match picked.await {
            Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
            Ok(Err(error)) => Err(log::fail("File dialog failed", format!("{error:#}")).into()),
            Err(_) => Ok(None),
        }
    }
}

pub(super) fn pick_file(prompt: &str, cx: &App) -> impl Future<Output = Picked> + use<> {
    pick(false, Some(prompt), cx)
}

pub(super) fn pick_folder(cx: &App) -> impl Future<Output = Picked> + use<> {
    pick(true, None, cx)
}
