//! Global push-to-talk shortcut (M2): hold the shortcut to talk, release to send.
//!
//! The shortcut comes from `settings.voice.ptt_shortcut` and is only registered while voice
//! is enabled, so Tandem never claims a key combination the user isn't using. It is
//! re-registered whenever the core emits new settings. Registration happens in Rust, so the
//! WebView gets no global-shortcut permissions at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tandem_core::events::{CoreEvent, CoreNotice};
use tandem_core::settings::VoiceSettings;
use tandem_core::Core;
use tauri::async_runtime::{self, Sender};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// i18n key for the gentle notice shown when the shortcut can't be registered.
const NOTICE_TAKEN: &str = "voice.shortcut_taken";

#[derive(Debug, Clone, Copy)]
enum Ptt {
    Press,
    Release,
}

/// App-managed state for the push-to-talk shortcut.
pub struct PttShortcut {
    /// The shortcut currently registered (None = nothing registered).
    current: Mutex<Option<Shortcut>>,
    /// True between press and release; filters out key-repeat presses.
    held: AtomicBool,
    /// Presses and releases go through one worker so `ptt_stop` never overtakes `ptt_start`.
    tx: Sender<Ptt>,
}

/// The global-shortcut plugin, with the push-to-talk handler attached.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| on_shortcut(app, shortcut, event))
        .build()
}

/// Call once in `setup`, after `Core` is managed.
pub fn init<R: Runtime>(app: &AppHandle<R>, core: Arc<Core>) {
    let (tx, mut rx) = async_runtime::channel::<Ptt>(16);
    async_runtime::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            let result = match cmd {
                Ptt::Press => core.voice().ptt_start().await,
                Ptt::Release => core.voice().ptt_stop().await.map(|_| ()),
            };
            // The controller reports problems on `voice://state`; nothing else to do here.
            if let Err(e) = result {
                tracing::debug!("push-to-talk {cmd:?}: {e}");
            }
        }
    });
    app.manage(PttShortcut {
        current: Mutex::new(None),
        held: AtomicBool::new(false),
        tx,
    });
}

fn on_shortcut<R: Runtime>(app: &AppHandle<R>, shortcut: &Shortcut, event: ShortcutEvent) {
    let Some(state) = app.try_state::<PttShortcut>() else {
        return;
    };
    let is_ptt = matches!(state.current.lock(), Ok(g) if g.as_ref() == Some(shortcut));
    if !is_ptt {
        return;
    }
    let cmd = match event.state {
        ShortcutState::Pressed => {
            if state.held.swap(true, Ordering::SeqCst) {
                return; // key repeat
            }
            Ptt::Press
        }
        ShortcutState::Released => {
            if !state.held.swap(false, Ordering::SeqCst) {
                return;
            }
            Ptt::Release
        }
    };
    if let Err(e) = state.tx.try_send(cmd) {
        tracing::warn!("push-to-talk queue full: {e}");
    }
}

/// The shortcut that should be registered for these settings, if any.
fn desired(voice: &VoiceSettings) -> Option<std::result::Result<Shortcut, String>> {
    let text = voice.ptt_shortcut.trim();
    if !voice.enabled || text.is_empty() {
        return None;
    }
    Some(text.parse::<Shortcut>().map_err(|e| e.to_string()))
}

/// Register (or re-register) the shortcut for the given settings. Cheap when nothing changed.
/// The plugin blocks on the main thread, so the work runs on a blocking worker.
pub fn apply<R: Runtime>(app: &AppHandle<R>, voice: &VoiceSettings) {
    let app = app.clone();
    let voice = voice.clone();
    async_runtime::spawn_blocking(move || apply_now(&app, &voice));
}

fn apply_now<R: Runtime>(app: &AppHandle<R>, voice: &VoiceSettings) {
    let Some(state) = app.try_state::<PttShortcut>() else {
        return;
    };
    let Ok(mut current) = state.current.lock() else {
        return;
    };
    let wanted = desired(voice);
    if let (Some(Ok(w)), Some(c)) = (&wanted, current.as_ref()) {
        if w == c {
            return;
        }
    }
    if wanted.is_none() && current.is_none() {
        return;
    }
    let shortcuts = app.global_shortcut();
    if let Some(old) = current.take() {
        if let Err(e) = shortcuts.unregister(old) {
            tracing::warn!("couldn't unregister push-to-talk shortcut: {e}");
        }
    }
    state.held.store(false, Ordering::SeqCst);
    match wanted {
        None => {}
        Some(Ok(shortcut)) => match shortcuts.register(shortcut) {
            Ok(()) => *current = Some(shortcut),
            Err(e) => notify_taken(app, &voice.ptt_shortcut, &e.to_string()),
        },
        Some(Err(e)) => notify_taken(app, &voice.ptt_shortcut, &e),
    }
}

fn notify_taken<R: Runtime>(app: &AppHandle<R>, shortcut: &str, error: &str) {
    tracing::warn!("push-to-talk shortcut '{shortcut}' unavailable: {error}");
    let event = CoreEvent::Notice(CoreNotice {
        key: NOTICE_TAKEN.into(),
        detail: Some(shortcut.to_string()),
    });
    if let Err(e) = app.emit(event.channel(), event.payload()) {
        tracing::warn!("failed to emit notice: {e}");
    }
}
