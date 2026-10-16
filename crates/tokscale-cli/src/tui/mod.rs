mod app;
mod cache;
pub mod client_ui;
pub(crate) mod codex_login;
mod colors;
pub mod config;
pub mod data;
mod event;
mod export;
pub(crate) mod glyph_probe;
pub mod i18n;
mod keymap;
pub(crate) mod privacy;
pub mod remote;
pub mod settings;
mod themes;
pub(crate) mod ui;

pub use app::{App, Tab, TuiConfig};
pub use cache::{
    load_cache, save_cached_data, CacheReportScope, CacheResult, TUI_DEFAULT_GROUP_BY,
};
pub use data::{DataLoader, UsageData};
pub use event::{Event, EventHandler};

use std::collections::HashSet;
use std::io;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread::{self, JoinHandle};
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
use tokscale_core::{ClientId, PendingSourceCacheSave};

use crate::ClientFilter;

const BACKGROUND_LOADER_THREAD_NAME: &str = "tokscale-tui-data-loader";

fn panic_should_restore_terminal(thread_name: Option<&str>) -> bool {
    thread_name != Some(BACKGROUND_LOADER_THREAD_NAME)
}

fn decide_initial_data(load_result: CacheResult) -> (Option<UsageData>, bool) {
    let cached_data = match load_result {
        CacheResult::Fresh(data) | CacheResult::Stale(data) => Some(data),
        CacheResult::Miss => None,
    };

    (cached_data, true)
}

fn background_data_loader(
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    minutely_enabled: bool,
) -> DataLoader {
    DataLoader::with_filters(None, since, until, year)
        .with_minutely_enabled(minutely_enabled)
        // The Projects tab rolls up workspaces regardless of the global
        // grouping, so both background loads always pay workspace resolution.
        .with_projects_enabled(true)
}

fn background_cache_scope(
    since: &Option<String>,
    until: &Option<String>,
    year: &Option<String>,
) -> CacheReportScope {
    CacheReportScope::new(since.clone(), until.clone(), year.clone())
}

enum BackgroundLoadEvent {
    Data(Result<Box<UsageData>>),
    Persisted,
}

fn publish_then_persist<P>(
    tx: &mpsc::Sender<BackgroundLoadEvent>,
    data: UsageData,
    pending_save: P,
    before_publish: impl FnOnce(&UsageData),
    persist: impl FnOnce(P),
) {
    before_publish(&data);
    let _ = tx.send(BackgroundLoadEvent::Data(Ok(Box::new(data))));
    // This runs even when the event loop has gone away. It is deliberately
    // synchronous on the existing loader thread: there is no detached cache
    // worker whose lifetime could escape the owned JoinHandle.
    persist(pending_save);
}

fn spawn_background_loader(
    tx: mpsc::Sender<BackgroundLoadEvent>,
    load: impl FnOnce() -> Result<(UsageData, PendingSourceCacheSave)> + Send + 'static,
    before_publish: impl FnOnce(&UsageData) + Send + 'static,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name(BACKGROUND_LOADER_THREAD_NAME.to_string())
        .spawn(move || {
            match load() {
                Ok((data, pending_save)) => publish_then_persist(
                    &tx,
                    data,
                    pending_save,
                    before_publish,
                    PendingSourceCacheSave::persist,
                ),
                Err(error) => {
                    let _ = tx.send(BackgroundLoadEvent::Data(Err(error)));
                }
            }
            // Ordered after Data and, on success, after synchronous persistence.
            // This event is the generation boundary used by the event loop.
            let _ = tx.send(BackgroundLoadEvent::Persisted);
        })
        .expect("failed to spawn TUI background loader")
}

#[derive(Default)]
struct BackgroundLoadState {
    handle: Option<JoinHandle<()>>,
    in_flight: bool,
    data_ready: bool,
}

impl BackgroundLoadState {
    fn can_start(&self) -> bool {
        !self.in_flight && self.handle.is_none()
    }

    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.in_flight
    }

    fn start(&mut self, handle: JoinHandle<()>) {
        assert!(self.can_start(), "background loader already active");
        self.handle = Some(handle);
        self.in_flight = true;
        self.data_ready = false;
    }

    fn mark_persisted(&mut self) -> bool {
        let data_ready = self.data_ready;
        self.data_ready = false;
        self.in_flight = false;
        data_ready
    }

    /// Join only after the worker has reached terminal state.
    fn reap_finished(&mut self) -> Option<thread::Result<()>> {
        if !self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            return None;
        }
        Some(self.handle.take().expect("finished handle exists").join())
    }

    /// Apply the explicit-quit policy without moving cache latency to shutdown.
    fn finish_for_exit(&mut self) -> Option<thread::Result<()>> {
        let result = if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            self.reap_finished()
        } else {
            // Dropping a running handle does not start another thread or wait.
            // The CLI process exits immediately after terminal restoration.
            self.handle.take();
            None
        };
        self.in_flight = false;
        self.data_ready = false;
        result
    }
}

fn drain_background_events(
    app: &mut App,
    state: &mut BackgroundLoadState,
    rx: &mpsc::Receiver<BackgroundLoadEvent>,
) {
    loop {
        match rx.try_recv() {
            Ok(BackgroundLoadEvent::Data(result)) => {
                app.set_background_loading(false);
                match result {
                    Ok(data) => {
                        app.update_data(*data);
                        app.source_cache_persisting = true;
                        state.data_ready = true;
                    }
                    Err(error) => {
                        app.source_cache_persisting = false;
                        state.data_ready = false;
                        app.set_error(Some(error.to_string()));
                        app.set_status(&format!("Error: {error}"));
                    }
                }
            }
            Ok(BackgroundLoadEvent::Persisted) => {
                app.set_background_loading(false);
                app.source_cache_persisting = false;
                if state.mark_persisted() {
                    app.set_status("Data loaded");
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                if state.in_flight {
                    state.in_flight = false;
                    state.data_ready = false;
                    app.set_background_loading(false);
                    app.source_cache_persisting = false;
                    app.set_error(Some("Background thread disconnected".to_string()));
                    app.set_status("Error: Background thread disconnected");
                }
                break;
            }
        }
    }
}

/// Drain in channel order on both sides of handle reaping. If a worker sends
/// `Data` and `Persisted` between the first Empty and `is_finished`, joining
/// establishes that both sends completed and the second drain consumes them
/// before any queued reload can start.
fn advance_background_load(
    app: &mut App,
    state: &mut BackgroundLoadState,
    rx: &mpsc::Receiver<BackgroundLoadEvent>,
) {
    drain_background_events(app, state, rx);
    let join_result = state.reap_finished();
    drain_background_events(app, state, rx);

    if let Some(join_result) = join_result {
        if join_result.is_err() || state.in_flight {
            state.in_flight = false;
            state.data_ready = false;
            app.set_background_loading(false);
            app.source_cache_persisting = false;
            let message = if join_result.is_err() {
                "Background thread panicked"
            } else {
                "Background thread completed without persistence confirmation"
            };
            app.set_error(Some(message.to_string()));
            app.set_status(&format!("Error: {message}"));
        }
    }
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
    worktree_rollup: tokscale_core::WorktreeRollup,
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
        worktree_rollup,
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
    // The key MUST be `cache::TUI_DEFAULT_GROUP_BY` — any code path that
    // writes the TUI cache (notably `run_warm_tui_cache` in main.rs) keys
    // on the same constant. Hard-coding a different value here would
    // silently invalidate the cache on every launch after `submit`.
    let initial_group_by = TUI_DEFAULT_GROUP_BY;
    let initial_report_scope = background_cache_scope(&since, &until, &year);
    let (cached_data, needs_background_load) = decide_initial_data(load_cache(
        &enabled_clients,
        &initial_group_by,
        &initial_report_scope,
    ));

    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // The event loop owns loader failures through JoinHandle::join. Let it
        // keep the terminal intact and render the error instead of invoking the
        // process-wide terminal/stderr panic path from that worker.
        if panic_should_restore_terminal(thread::current().name()) {
            restore_terminal_best_effort();
            original_hook(info);
        }
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    let _ = execute!(stdout, SetTitle("Tokscale"));

    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout, SetTitle(""));
        return Err(e.into());
    }

    // Forced modes need no capability query. Auto probes only while raw mode
    // and the alternate screen are active; the probe is erased before the
    // first frame and degrades to Unknown when DSR is unavailable.
    let probe = if settings::Settings::load().glyphs == settings::GlyphMode::Auto {
        glyph_probe::probe_ambient_width(&mut stdout)
    } else {
        glyph_probe::ProbeResult::default()
    };
    let ambient_width = probe.width;

    // Mouse reporting starts only after the input-side probe. A physical click
    // on the still-blank alternate screen must not become an escape sequence
    // mixed into the cursor-position response.
    if let Err(error) = execute!(stdout, EnableMouseCapture) {
        restore_terminal_best_effort();
        return Err(error.into());
    }

    // From here on the TUI owns raw mode and the alternate screen for its
    // whole run. Background diagnostics (e.g. cache save warnings) must not
    // write raw stdio while this is set, or they corrupt the rendered
    // display instead of being visible as a log line. Cleared in both
    // `restore_terminal` and `restore_terminal_best_effort` below.
    tokscale_core::tui_signal::set_tui_active(true);

    let backend = CrosstermBackend::new(stdout);
    let terminal_result = Terminal::new(backend);
    let mut terminal = match terminal_result {
        Ok(t) => t,
        Err(e) => {
            restore_terminal_best_effort();
            return Err(e.into());
        }
    };

    let mut app =
        match App::new_with_cached_data_and_ambient_width(config, cached_data, ambient_width) {
            Ok(a) => a,
            Err(e) => {
                restore_terminal(&mut terminal);
                return Err(e);
            }
        };

    // Cache-first load of server-side aggregated multi-device stats. The
    // background refresh (when the cache is stale or missing) is driven by
    // App::on_tick, and every failure path degrades silently to local-only.
    app.init_remote_stats();

    let (bg_tx, bg_rx) = mpsc::channel::<BackgroundLoadEvent>();
    let mut background_loader = BackgroundLoadState::default();

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
        let cache_group_by = bg_group_by.clone();
        let bg_report_scope = background_cache_scope(&since, &until, &year);
        let bg_minutely_enabled = app.settings.minutely_tab_enabled;

        background_loader.start(spawn_background_loader(
            tx,
            move || {
                let loader =
                    background_data_loader(bg_since, bg_until, bg_year, bg_minutely_enabled);
                loader.load_deferred(&bg_clients, &bg_group_by, bg_include_synthetic)
            },
            move |data| {
                save_cached_data(data, &bg_enabled_clients, &cache_group_by, &bg_report_scope);
            },
        ));
    }

    #[cfg(unix)]
    let sigcont_flag = {
        let flag = Arc::new(AtomicBool::new(false));
        let _ = signal_hook::flag::register(signal_hook::consts::SIGCONT, Arc::clone(&flag));
        flag
    };

    let mut events =
        EventHandler::new_with_pending_keys(Duration::from_millis(100), probe.pending_keys);

    let result = run_loop_with_background(
        &mut terminal,
        &mut app,
        &mut events,
        bg_tx,
        bg_rx,
        background_loader,
        #[cfg(unix)]
        &sigcont_flag,
    );

    // Don't orphan a `codex login` child (it would keep holding the OAuth
    // port after the TUI exits).
    app.kill_codex_login_child();

    restore_terminal(&mut terminal);

    result
}

fn restore_terminal_best_effort() {
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        SetTitle("")
    );
    let _ = disable_raw_mode();
    // Flush diagnostics that were deferred while the TUI owned raw mode only
    // after normal stderr is visible again.
    tokscale_core::tui_signal::set_tui_active(false);
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
    // set_tui_active(false) drains deferred stderr, so it must be the last
    // restoration step rather than writing into the alternate screen.
    tokscale_core::tui_signal::set_tui_active(false);
}

fn run_loop_with_background(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventHandler,
    bg_tx: mpsc::Sender<BackgroundLoadEvent>,
    bg_rx: mpsc::Receiver<BackgroundLoadEvent>,
    mut background_loader: BackgroundLoadState,
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

        advance_background_load(app, &mut background_loader, &bg_rx);

        if app.needs_reload && background_loader.can_start() {
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
            let report_scope = background_cache_scope(&since, &until, &year);
            let minutely_enabled = app.settings.minutely_tab_enabled;
            let worktree_rollup = app.worktree_rollup;

            let cache_group_by = group_by.clone();
            background_loader.start(spawn_background_loader(
                tx,
                move || {
                    let loader = background_data_loader(since, until, year, minutely_enabled)
                        .with_worktree_rollup(worktree_rollup);
                    loader.load_deferred(&clients, &group_by, include_synthetic)
                },
                move |data| {
                    // Rolled-up rows are a different aggregation of the same
                    // messages, so caching them under the plain workspace scope
                    // would serve merged rows to a later un-merged view.
                    if worktree_rollup == tokscale_core::WorktreeRollup::Separate {
                        save_cached_data(data, &enabled_clients, &cache_group_by, &report_scope);
                    }
                },
            ));
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
    // Never move an unfinished cold-cache tail onto explicit quit. A handle
    // that raced to completion is cheap to reap; an active one is abandoned
    // so the caller can restore the terminal and return immediately.
    let _ = background_loader.finish_for_exit();
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
        ClientId::Freebuff,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_hook_restores_terminal_except_for_owned_loader_thread() {
        assert!(panic_should_restore_terminal(None));
        assert!(panic_should_restore_terminal(Some("main")));
        assert!(panic_should_restore_terminal(Some("tokscale-test-worker")));
        assert!(!panic_should_restore_terminal(Some(
            BACKGROUND_LOADER_THREAD_NAME
        )));
    }

    #[test]
    fn spawned_loader_panic_becomes_event_loop_error_without_join_wait() {
        let (event_tx, event_rx) = mpsc::channel();
        let mut state = BackgroundLoadState::default();
        let mut app = background_event_test_app();
        state.start(spawn_background_loader(
            event_tx,
            || -> Result<(UsageData, PendingSourceCacheSave)> {
                panic!("synthetic loader panic");
            },
            |_| {},
        ));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while state.in_flight || state.handle.is_some() {
            advance_background_load(&mut app, &mut state, &event_rx);
            assert!(
                std::time::Instant::now() < deadline,
                "panicked loader was not reaped"
            );
            thread::yield_now();
        }

        assert!(!app.background_loading);
        assert!(!app.source_cache_persisting);
        assert_eq!(
            app.data.error.as_deref(),
            Some("Background thread panicked")
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("Error: Background thread panicked")
        );
    }

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

    #[test]
    fn background_loader_preserves_minutely_toggle() {
        let enabled = background_data_loader(None, None, None, true);
        assert!(enabled.minutely_enabled);

        let disabled = background_data_loader(None, None, None, false);
        assert!(!disabled.minutely_enabled);
    }

    #[test]
    fn background_loader_enables_projects() {
        // The Projects tab rolls up workspaces regardless of the global
        // grouping, so both background loads must resolve workspaces.
        let loader = background_data_loader(None, None, None, false);
        assert!(loader.projects_enabled);
    }

    fn background_event_test_app() -> App {
        App::new_with_cached_data(
            TuiConfig {
                theme: "blue".to_string(),
                ..Default::default()
            },
            Some(UsageData::default()),
        )
        .unwrap()
    }

    #[test]
    fn auto_glyphs_follow_the_probe_and_favor_unicode_when_unknown() {
        for (width, expected_corner) in [
            (glyph_probe::AmbientWidth::Narrow, "┌"),
            (glyph_probe::AmbientWidth::Wide, "+"),
            (glyph_probe::AmbientWidth::Unknown, "┌"),
        ] {
            let app = App::new_with_cached_data_and_ambient_width(
                TuiConfig::default(),
                Some(UsageData::default()),
                width,
            )
            .unwrap();
            assert_eq!(app.glyphs.border.top_left, expected_corner);
        }
    }

    #[test]
    fn publishes_aggregate_after_cache_callback_and_before_source_save() {
        let (data_tx, data_rx) = mpsc::channel::<BackgroundLoadEvent>();
        let (order_tx, order_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let order_tx_for_cache = order_tx.clone();

        let handle = thread::spawn(move || {
            publish_then_persist(
                &data_tx,
                UsageData::default(),
                (),
                move |_| order_tx_for_cache.send("cache").unwrap(),
                move |_| {
                    order_tx.send("persist").unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                },
            );
        });

        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "cache"
        );
        assert!(matches!(
            data_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            BackgroundLoadEvent::Data(Ok(_))
        ));
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "persist"
        );
        assert!(!handle.is_finished());
        release_tx.send(()).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn dropped_ui_receiver_does_not_skip_source_save() {
        let (data_tx, data_rx) = mpsc::channel::<BackgroundLoadEvent>();
        drop(data_rx);
        let (save_entered_tx, save_entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            publish_then_persist(
                &data_tx,
                UsageData::default(),
                (),
                |_| {},
                move |_| {
                    save_entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                },
            );
        });

        save_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        release_tx.send(()).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn data_event_keeps_reload_coalesced_until_persisted_event() {
        let (event_tx, event_rx) = mpsc::channel();
        let (save_entered_tx, save_entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut state = BackgroundLoadState::default();
        let mut app = background_event_test_app();
        app.needs_reload = true;

        state.start(thread::spawn(move || {
            event_tx
                .send(BackgroundLoadEvent::Data(Ok(
                    Box::new(UsageData::default()),
                )))
                .unwrap();
            save_entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            event_tx.send(BackgroundLoadEvent::Persisted).unwrap();
        }));

        save_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        advance_background_load(&mut app, &mut state, &event_rx);
        assert!(state.is_active());
        assert!(
            !state.can_start(),
            "queued reload started before persistence"
        );
        assert!(app.source_cache_persisting);
        assert!(app.needs_reload);

        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !state.can_start() {
            advance_background_load(&mut app, &mut state, &event_rx);
            assert!(
                std::time::Instant::now() < deadline,
                "loader did not reach persisted state"
            );
            thread::yield_now();
        }
        assert!(!state.is_active());
        assert!(!app.source_cache_persisting);
        assert!(app.needs_reload, "one coalesced reload remains queued");
    }

    #[test]
    fn fast_data_and_persisted_events_are_drained_before_next_loader() {
        let (event_tx, event_rx) = mpsc::channel();
        let (events_sent_tx, events_sent_rx) = mpsc::channel();
        let mut state = BackgroundLoadState::default();
        let mut app = background_event_test_app();
        app.needs_reload = true;

        state.start(thread::spawn(move || {
            let data = UsageData {
                total_tokens: 42,
                ..UsageData::default()
            };
            event_tx
                .send(BackgroundLoadEvent::Data(Ok(Box::new(data))))
                .unwrap();
            event_tx.send(BackgroundLoadEvent::Persisted).unwrap();
            events_sent_tx.send(()).unwrap();
        }));
        events_sent_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        advance_background_load(&mut app, &mut state, &event_rx);
        assert_eq!(app.data.total_tokens, 42);
        assert!(!state.is_active());
        assert!(!app.source_cache_persisting);
        assert_eq!(app.status_message.as_deref(), Some("Data loaded"));
        // Even after Persisted, a worker handle that has not reached terminal
        // state prevents a replacement from overwriting its generation.
        if state.handle.is_some() {
            assert!(!state.can_start());
        }

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !state.can_start() {
            advance_background_load(&mut app, &mut state, &event_rx);
            assert!(
                std::time::Instant::now() < deadline,
                "finished generation was not reaped"
            );
            thread::yield_now();
        }
        assert!(app.needs_reload);
    }

    #[test]
    fn explicit_exit_joins_loader_that_is_already_finished() {
        let mut loader = BackgroundLoadState::default();
        loader.start(thread::spawn(|| {}));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !loader.handle.as_ref().expect("loader handle").is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "loader did not finish"
            );
            thread::yield_now();
        }

        assert!(loader
            .finish_for_exit()
            .expect("finished handle reaped")
            .is_ok());
        assert!(!loader.is_active());
    }

    #[test]
    fn explicit_exit_drops_unfinished_loader_handle_without_joining() {
        let (save_entered_tx, save_entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let mut loader = BackgroundLoadState::default();
        loader.start(thread::spawn(move || {
            save_entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            done_tx.send(()).unwrap();
        }));

        save_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(loader.finish_for_exit().is_none());
        assert!(!loader.is_active());
        assert!(matches!(done_rx.try_recv(), Err(TryRecvError::Empty)));

        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn background_cache_scope_uses_date_filters() {
        let scope = background_cache_scope(
            &Some("2026-05-01".to_string()),
            &Some("2026-05-07".to_string()),
            &Some("2026".to_string()),
        );

        assert_eq!(
            scope,
            crate::tui::cache::CacheReportScope::new(
                Some("2026-05-01".to_string()),
                Some("2026-05-07".to_string()),
                Some("2026".to_string()),
            )
        );
    }
}
