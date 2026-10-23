mod brief;
mod tmux;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(tmux::Tmux::default())
        .invoke_handler(tauri::generate_handler![
            tmux::list_panes,
            tmux::capture,
            tmux::attach,
            tmux::send_keys,
            tmux::resize,
            brief::brief,
            brief::changes,
            brief::card,
            brief::approve_plan,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
