//! Tauri shell: hosts the WebViews, forwards core events, owns the tray, and registers
//! commands. All business logic lives in the `core` crate.

mod ptt;

use std::sync::Arc;

use tandem_core::events::{CoreEvent, EventSink};
use tandem_core::secrets::KeyringStore;
use tandem_core::windows::{self, BUDDY, MAIN};
use tandem_core::Core;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

/// User-facing strings come from the UI's i18n file so there is one source of copy.
const EN: &str = include_str!("../../../../ui/src/i18n/en.json");

fn text(key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(EN)
        .ok()
        .and_then(|v| v.get(key).and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_else(|| key.to_string())
}

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: CoreEvent) {
        // New settings may change the push-to-talk shortcut (or turn voice on/off).
        if let CoreEvent::Settings(settings) = &event {
            ptt::apply(&self.0, &settings.voice);
        }
        if let Err(e) = self.0.emit(event.channel(), event.payload()) {
            tracing::warn!("failed to emit {}: {e}", event.channel());
        }
    }
}

fn core(app: &AppHandle) -> Arc<Core> {
    app.state::<Arc<Core>>().inner().clone()
}

fn toggle_buddy(app: &AppHandle) {
    let core = core(app);
    let enabled = !core.settings().buddy.enabled;
    match core.set_buddy_enabled(enabled) {
        Ok(settings) => windows::apply_buddy(app, &settings.buddy),
        Err(e) => tracing::warn!("couldn't toggle buddy: {e}"),
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", text("tray.open"), true, None::<&str>)?;
    let buddy = MenuItem::with_id(app, "buddy", text("tray.toggle_buddy"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", text("tray.quit"), true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &buddy, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("tandem")
        .tooltip(text("app.title"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => windows::show_main(app),
            "buddy" => toggle_buddy(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                windows::show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tandem_core=info".into()),
        )
        .init();

    let result = tauri::Builder::default()
        .plugin(ptt::plugin())
        .invoke_handler(tandem_core::commands::handler())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let sink = Arc::new(TauriSink(app.handle().clone()));
            let core = tauri::async_runtime::block_on(async move {
                let core = Core::init(&data_dir, sink, Arc::new(KeyringStore)).await?;
                core.spawn_ticker();
                Ok::<_, tandem_core::CoreError>(core)
            })?;
            let settings = core.settings();
            ptt::init(app.handle(), core.clone());
            app.manage(core);
            ptt::apply(app.handle(), &settings.voice);
            let buddy = settings.buddy;
            build_tray(app.handle())?;
            windows::apply_buddy(app.handle(), &buddy);
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match (window.label(), event) {
                // With the buddy on the desktop, closing the main window just hides it;
                // quit from the tray. Without the buddy, closing quits as usual.
                (MAIN, WindowEvent::CloseRequested { api, .. }) => {
                    if core(app).settings().buddy.enabled {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        app.exit(0);
                    }
                }
                (BUDDY, WindowEvent::Moved(pos)) => {
                    let core = core(app);
                    let (x, y) = (pos.x, pos.y);
                    // Window events arrive outside the async runtime.
                    tauri::async_runtime::spawn(async move { core.save_buddy_position(x, y) });
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("Tandem failed to start: {e}");
        std::process::exit(1);
    }
}
