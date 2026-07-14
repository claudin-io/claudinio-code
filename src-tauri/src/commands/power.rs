use std::sync::Mutex;
use tauri::State;

#[derive(Default)]
pub struct KeepAwakeState(pub Mutex<Option<keepawake::KeepAwake>>);

#[tauri::command]
pub fn set_keep_awake(active: bool, state: State<KeepAwakeState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;

    if active && guard.is_none() {
        let awake = keepawake::Builder::default()
            .idle(true)
            .sleep(true)
            .reason("Agent session running")
            .app_name("Claudinio Code")
            .create();
        match awake {
            Ok(handle) => *guard = Some(handle),
            Err(e) => eprintln!("Failed to acquire keep-awake lock: {e}"),
        }
    } else if !active && guard.is_some() {
        *guard = None;
    }

    Ok(())
}
