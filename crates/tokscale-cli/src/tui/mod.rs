mod app;
mod cache;
pub mod client_ui;
mod colors;
pub mod config;
pub mod data;
mod event;
mod export;
pub mod settings;
mod themes;
mod ui;

pub use app::{App, Tab, TuiConfig};
pub use cache::{load_cache, save_cached_data, CacheResult};
pub use data::{DataLoader, UsageData};
pub use event::{Event, EventHandler};

use std::collections::HashSet;
use std::io;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::Arc;

use std::panic;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
    },
};
use ratatui::prelude::*;
use tokscale_core::ClientId;

use crate::ClientFilter;

fn decide_initial_data(load_result: CacheResult) -> (Option<UsageData>, bool) {
    let cached_data = match load_result {
        CacheResult::Fresh(data) | CacheResult::Stale(data) => Some(data),
        CacheResult::Miss => None,
    };

    (cached_data, true)
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    theme: &str,
    refresh: u64,
    debug: bool,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    initial_tab: Option<Tab>,
) -> Result<()> {
    if debug {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("debug")
            .try_init();
    }

    let config = TuiConfig {
        theme: theme.to_string(),
        refresh,
        sessions_path: None,
        clients: clients.clone(),
        since: since.clone(),
        until: until.clone(),
        year: year.clone(),
        initial_tab,
    };

    // Build the unified filter set used by the cache key, the App
    // constructor, and the background loader. We mirror the same
    // resolution rules App::new_with_cached_data uses so the cache
    // lookup and the in-app state always agree. Drift between them
    // makes every launch a stale-cache hit instead of a fresh one.
    let enabled_clients: HashSet<ClientFilter> = if let Some(ref cli_clients) = clients {
        cli_clients
            .iter()
            .filter_map(|s| ClientFilter::from_filter_str(&s.to_lowercase()))
            .collect()
    } else {
        ClientFilter::default_set()
    };

    // Single file read: load cache and check freshness in one pass.
    let initial_group_by = tokscale_core::GroupBy::Model;
    let (cached_data, needs_background_load) =
        decide_initial_data(load_cache(&enabled_clients, &initial_group_by));

    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal_best_effort();
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    let _ = execute!(stdout, SetTitle("Tokscale"));

    if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout, SetTitle(""));
        return Err(e.into());
    }

    let backend = CrosstermBackend::new(stdout);
    let terminal_result = Terminal::new(backend);
    let mut terminal = match terminal_result {
        Ok(t) => t,
        Err(e) => {
            restore_terminal_best_effort();
            return Err(e.into());
        }
    };

    let mut app = match App::new_with_cached_data(config, cached_data) {
        Ok(a) => a,
        Err(e) => {
            restore_terminal(&mut terminal);
            return Err(e);
        }
    };

    let (bg_tx, bg_rx) = mpsc::channel::<Result<UsageData>>();

    if needs_background_load {
        app.set_background_loading(true);

        let tx = bg_tx.clone();
        // Project the filter set into the (clients, include_synthetic)
        // pair the loader still consumes. Keeping the projection here
        // (instead of inside DataLoader) avoids touching tokscale-core's
        // public API in this PR.
        let bg_clients: Vec<ClientId> = enabled_clients
            .iter()
            .filter_map(|f| f.to_client_id())
            .collect();
        let bg_include_synthetic = enabled_clients.contains(&ClientFilter::Synthetic);
        let bg_since = since.clone();
        let bg_until = until.clone();
        let bg_year = year.clone();
        let bg_enabled_clients = enabled_clients.clone();
        let bg_group_by = app.group_by.borrow().clone();

        thread::spawn(move || {
            let loader = DataLoader::with_filters(None, bg_since, bg_until, bg_year);
            let result = loader.load(&bg_clients, &bg_group_by, bg_include_synthetic);

            if let Ok(ref data) = result {
                save_cached_data(data, &bg_enabled_clients, &bg_group_by);
            }

            let _ = tx.send(result);
        });
    }

    #[cfg(unix)]
    let sigcont_flag = {
        let flag = Arc::new(AtomicBool::new(false));
        let _ = signal_hook::flag::register(signal_hook::consts::SIGCONT, Arc::clone(&flag));
        flag
    };

    let mut events = EventHandler::new(Duration::from_millis(100));

    let result = run_loop_with_background(
        &mut terminal,
        &mut app,
        &mut events,
        bg_tx,
        bg_rx,
        #[cfg(unix)]
        &sigcont_flag,
    );

    restore_terminal(&mut terminal);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launches_with_24h_old_cache_renders_immediately() {
        let (cached_data, needs_background_load) =
            decide_initial_data(CacheResult::Stale(UsageData::default()));

        assert!(cached_data.is_some());
        assert!(needs_background_load);
    }

    #[test]
    fn miss_renders_empty_until_background_completes() {
        let (cached_data, needs_background_load) = decide_initial_data(CacheResult::Miss);

        assert!(cached_data.is_none());
        assert!(needs_background_load);
    }
}

fn restore_terminal_best_effort() {
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        SetTitle("")
    );
    let _ = disable_raw_mode();
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) {
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        SetTitle("")
    );
    let _ = terminal.show_cursor();
}

fn run_loop_with_background(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventHandler,
    bg_tx: mpsc::Sender<Result<UsageData>>,
    bg_rx: mpsc::Receiver<Result<UsageData>>,
    #[cfg(unix)] sigcont_flag: &Arc<AtomicBool>,
) -> Result<()> {
    loop {
        #[cfg(unix)]
        if sigcont_flag.swap(false, Ordering::Relaxed) {
            let _ = enable_raw_mode();
            let _ = execute!(
                terminal.backend_mut(),
                EnterAlternateScreen,
                EnableMouseCapture
            );
            let _ = terminal.clear();
        }

        terminal.draw(|f| ui::render(f, app))?;

        match bg_rx.try_recv() {
            Ok(result) => {
                app.set_background_loading(false);
                match result {
                    Ok(data) => {
                        app.update_data(data);
                        app.set_status("Data loaded");
                    }
                    Err(e) => {
                        app.set_error(Some(e.to_string()));
                        app.set_status(&format!("Error: {}", e));
                    }
                }
            }
            Err(TryRecvError::Disconnected) => {
                if app.background_loading {
                    app.set_background_loading(false);
                    app.set_error(Some("Background thread disconnected".to_string()));
                    app.set_status("Error: Background thread disconnected");
                }
            }
            Err(TryRecvError::Empty) => {}
        }

        if app.needs_reload && !app.background_loading {
            app.needs_reload = false;
            app.set_background_loading(true);

            let tx = bg_tx.clone();
            // Boundary projection: see [`run`] above for the shape rationale.
            let clients = app.scan_clients();
            let include_synthetic = app.include_synthetic();
            let since = app.data_loader.since.clone();
            let until = app.data_loader.until.clone();
            let year = app.data_loader.year.clone();
            let enabled_clients = app.enabled_clients.borrow().clone();
            let group_by = app.group_by.borrow().clone();

            thread::spawn(move || {
                let loader = DataLoader::with_filters(None, since, until, year);
                let result = loader.load(&clients, &group_by, include_synthetic);
                if let Ok(ref data) = result {
                    save_cached_data(data, &enabled_clients, &group_by);
                }
                let _ = tx.send(result);
            });
        }

        match events.next()? {
            Event::Tick => {
                app.on_tick();
            }
            Event::Key(key) => {
                if app.handle_key_event(key) {
                    break;
                }
            }
            Event::Mouse(mouse) => {
                app.handle_mouse_event(mouse);
            }
            Event::Resize(w, h) => {
                app.handle_resize(w, h);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

pub fn test_data_loading() -> Result<()> {
    println!("Testing data loading...");

    let loader = DataLoader::new(None);
    let all_clients = vec![
        ClientId::OpenCode,
        ClientId::Claude,
        ClientId::Cursor,
        ClientId::Gemini,
        ClientId::Codex,
        ClientId::Amp,
        ClientId::Droid,
        ClientId::OpenClaw,
        ClientId::Pi,
        ClientId::Kimi,
        ClientId::Qwen,
        ClientId::RooCode,
        ClientId::KiloCode,
        ClientId::Kilo,
        ClientId::Mux,
        ClientId::Crush,
        ClientId::Hermes,
        ClientId::Codebuff,
    ];

    let data = loader.load(&all_clients, &tokscale_core::GroupBy::default(), false)?;

    println!("Loaded {} models", data.models.len());
    println!("Total cost: ${:.2}", data.total_cost);

    println!("\nAll models (client:model):");
    let mut models = data.models.clone();
    models.sort_by(|a, b| {
        let client_cmp = a.client.cmp(&b.client);
        if client_cmp == std::cmp::Ordering::Equal {
            a.model.cmp(&b.model)
        } else {
            client_cmp
        }
    });
    for m in &models {
        println!("{}:{}", m.client.to_lowercase(), m.model);
    }

    Ok(())
}
