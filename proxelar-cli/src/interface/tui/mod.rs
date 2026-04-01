mod event;
mod handler;
mod state;
mod ui;

use std::io::{self, Write};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use proxyapi::ProxyEvent;
use ratatui::prelude::*;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use event::{spawn_event_loop, AppEvent};
use handler::handle_key_event;
use state::{AppState, PendingAction};
use ui::draw;

/// Guard that restores the terminal on drop, even during panics.
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub async fn run(event_rx: mpsc::Receiver<ProxyEvent>, cancel: CancellationToken) {
    if let Err(e) = run_inner(event_rx, cancel).await {
        eprintln!("TUI error: {e}");
    }
}

async fn run_inner(
    event_rx: mpsc::Receiver<ProxyEvent>,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let _guard = RawModeGuard;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new();
    let mut app_events = spawn_event_loop(event_rx);

    loop {
        let event = tokio::select! {
            event = app_events.recv() => match event {
                Some(e) => e,
                None => break,
            },
            () = cancel.cancelled() => break,
        };

        match event {
            AppEvent::Input(key_event) => {
                if handle_key_event(key_event, &mut state) {
                    break;
                }
                if let Some(PendingAction::OpenEditor) = state.pending_action.take() {
                    open_in_editor(&mut terminal, &state).await;
                }
            }
            AppEvent::Proxy(proxy_event) => {
                state.add_event(proxy_event);
            }
            AppEvent::Render => {
                terminal.draw(|f| draw(f, &mut state))?;
            }
        }
    }

    // RawModeGuard handles cleanup on drop
    Ok(())
}

/// Suspend the TUI, open the selected response body in `$EDITOR`, then restore the TUI.
async fn open_in_editor(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &AppState) {
    use proxyapi::ProxyEvent;

    let (id, body, ext) = match state.selected_event() {
        Some(ProxyEvent::RequestComplete { id, response, .. }) => {
            let ext = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|ct| {
                    if ct.contains("json") {
                        "json"
                    } else if ct.contains("html") {
                        "html"
                    } else if ct.contains("xml") {
                        "xml"
                    } else {
                        "txt"
                    }
                })
                .unwrap_or("txt");
            (*id, response.body().to_owned(), ext)
        }
        _ => return,
    };

    let temp_path = std::env::temp_dir().join(format!("proxelar-response-{id}.{ext}"));

    if let Err(e) = std::fs::File::create(&temp_path).and_then(|mut f| f.write_all(&body)) {
        tracing::warn!("failed to write temp file for editor: {e}");
        return;
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());

    // Suspend TUI
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);

    let path = temp_path.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = std::process::Command::new(&editor).arg(&path).status();
    })
    .await;

    // Restore TUI
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen);
    let _ = terminal.clear();

    // Drain any input events buffered while the editor was running so they
    // are not misinterpreted as proxelar key bindings.
    while crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
        let _ = crossterm::event::read();
    }

    let _ = std::fs::remove_file(&temp_path);
}
