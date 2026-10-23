mod brief;
mod tmux;

use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{Runtime, Url};

/// Keeps the window on the app. A clicked link, say in an agent's answer,
/// opens in the system browser instead of replacing the app with the page.
fn links_outside<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::new("links-outside")
        .on_navigation(|_, url: &Url| {
            let app = url.scheme() == "tauri"
                || matches!(
                    url.host_str(),
                    Some("localhost" | "tauri.localhost" | "127.0.0.1")
                );
            if !app && matches!(url.scheme(), "http" | "https" | "mailto") {
                let _ = std::process::Command::new("xdg-open")
                    .arg(url.as_str())
                    .spawn();
            }
            app
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(links_outside())
        .manage(tmux::Tmux::default())
        .invoke_handler(tauri::generate_handler![
            tmux::list_panes,
            tmux::capture,
            tmux::attach,
            tmux::send_keys,
            tmux::resize,
            tmux::new_terminal,
            brief::brief,
            brief::changes,
            brief::card,
            brief::approve_plan,
            brief::start_review,
            brief::dispose,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
