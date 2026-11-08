//! Every Tauri command lives here (CLAUDE.md). Thin wrappers over `Core`.
//! The UI calls these only through `ui/src/ipc/commands.ts`; argument names are snake_case.

use std::sync::Arc;

use tauri::{AppHandle, Runtime, State};

use crate::app::{AppState, Core, ProviderStatus, ANTHROPIC_ACCOUNT};
use crate::audio::{Speaker, SttModelInfo, VoiceInfo, VoiceStatus};
use crate::conversation::{ChatMessageDto, Via};
use crate::error::{CoreError, Result};
use crate::session::{CheckinMode, SessionSnapshot};
use crate::settings::{Profile, Settings};
use crate::store::HistoryEntry;
use crate::windows;

type CoreState<'a> = State<'a, Arc<Core>>;

fn account(provider: &str) -> Result<&'static str> {
    match provider {
        "anthropic" => Ok(ANTHROPIC_ACCOUNT),
        other => Err(CoreError::Invalid(format!("unknown provider '{other}'"))),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_state(core: CoreState<'_>) -> AppState {
    core.state()
}

#[tauri::command(rename_all = "snake_case")]
pub fn start_session(
    core: CoreState<'_>,
    task: String,
    minutes: Option<u32>,
    checkin_mode: Option<CheckinMode>,
) -> Result<SessionSnapshot> {
    core.start_session(&task, minutes, checkin_mode)
}

#[tauri::command(rename_all = "snake_case")]
pub fn pause_session(core: CoreState<'_>) -> Result<SessionSnapshot> {
    core.pause_session()
}

#[tauri::command(rename_all = "snake_case")]
pub fn resume_session(core: CoreState<'_>) -> Result<SessionSnapshot> {
    core.resume_session()
}

#[tauri::command(rename_all = "snake_case")]
pub fn extend_session(core: CoreState<'_>, minutes: u32) -> Result<SessionSnapshot> {
    core.extend_session(minutes)
}

#[tauri::command(rename_all = "snake_case")]
pub fn end_session(core: CoreState<'_>, reflection: Option<String>) -> Result<SessionSnapshot> {
    core.end_session(reflection)
}

#[tauri::command(rename_all = "snake_case")]
pub fn close_session(core: CoreState<'_>) -> Result<SessionSnapshot> {
    core.close_session()
}

#[tauri::command(rename_all = "snake_case")]
pub fn start_break(core: CoreState<'_>, minutes: Option<u32>) -> Result<SessionSnapshot> {
    core.start_break(minutes)
}

#[tauri::command(rename_all = "snake_case")]
pub fn add_subtasks(core: CoreState<'_>, items: Vec<String>) -> Result<SessionSnapshot> {
    core.add_subtasks(&items)
}

#[tauri::command(rename_all = "snake_case")]
pub fn complete_subtask(core: CoreState<'_>, id: String) -> Result<SessionSnapshot> {
    core.complete_subtask(&id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_quiet(core: CoreState<'_>, minutes: u32) -> Result<SessionSnapshot> {
    core.set_quiet(minutes)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_checkin_mode(core: CoreState<'_>, mode: CheckinMode) -> SessionSnapshot {
    core.set_checkin_mode(mode)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn send_message(
    core: CoreState<'_>,
    text: String,
    via: Option<Via>,
) -> Result<Option<ChatMessageDto>> {
    core.send_message(&text, via.unwrap_or(Via::Text)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    core: CoreState<'_>,
    settings: Settings,
) -> Result<ProviderStatus> {
    let status = core.update_settings(settings).await?;
    windows::apply_buddy(&app, &core.settings().buddy);
    Ok(status)
}

#[tauri::command(rename_all = "snake_case")]
pub fn show_main_window<R: Runtime>(app: AppHandle<R>) {
    windows::show_main(&app);
}

/// Show or hide the desktop buddy and remember the choice.
#[tauri::command(rename_all = "snake_case")]
pub fn set_buddy_visible<R: Runtime>(
    app: AppHandle<R>,
    core: CoreState<'_>,
    visible: bool,
) -> Result<Settings> {
    let settings = core.set_buddy_enabled(visible)?;
    windows::apply_buddy(&app, &settings.buddy);
    Ok(settings)
}

#[tauri::command(rename_all = "snake_case")]
pub fn update_profile(core: CoreState<'_>, profile: Profile) -> Result<Profile> {
    core.update_profile(profile)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_api_key(
    core: CoreState<'_>,
    provider: String,
    key: String,
) -> Result<ProviderStatus> {
    core.set_api_key(account(&provider)?, &key).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn clear_api_key(core: CoreState<'_>, provider: String) -> Result<ProviderStatus> {
    core.clear_api_key(account(&provider)?).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_provider(core: CoreState<'_>) -> Result<ProviderStatus> {
    Ok(core.rebuild_providers().await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_ollama_models(core: CoreState<'_>, url: String) -> Result<Vec<String>> {
    core.list_ollama_models(&url).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn session_history(core: CoreState<'_>, limit: Option<u32>) -> Result<Vec<HistoryEntry>> {
    core.session_history(limit.unwrap_or(50))
}

// --- Voice (M2). Thin wrappers; all audio logic lives in `core/src/audio/`. ---

#[tauri::command(rename_all = "snake_case")]
pub fn voice_status(core: CoreState<'_>) -> VoiceStatus {
    core.voice().status()
}

/// Push-to-talk pressed (mic button or global shortcut). Stops companion speech first.
#[tauri::command(rename_all = "snake_case")]
pub async fn ptt_start(core: CoreState<'_>) -> Result<()> {
    core.voice().ptt_start().await
}

/// Push-to-talk released: transcribe and send. Returns the transcript ("" if nothing was heard).
#[tauri::command(rename_all = "snake_case")]
pub async fn ptt_stop(core: CoreState<'_>) -> Result<String> {
    core.voice().ptt_stop().await
}

#[tauri::command(rename_all = "snake_case")]
pub fn ptt_cancel(core: CoreState<'_>) {
    core.voice().ptt_cancel();
}

#[tauri::command(rename_all = "snake_case")]
pub fn stop_speaking(core: CoreState<'_>) {
    core.speaker().stop();
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_voices(core: CoreState<'_>) -> Vec<VoiceInfo> {
    core.speaker().voices()
}

/// Only ever called from an explicit Download click in Settings (rule 10).
#[tauri::command(rename_all = "snake_case")]
pub async fn download_voice(core: CoreState<'_>, id: String) -> Result<()> {
    core.speaker().download_voice(&id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn preview_voice(core: CoreState<'_>, id: String, text: String) -> Result<()> {
    core.speaker().preview_voice(&id, &text).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_stt_models(core: CoreState<'_>) -> Vec<SttModelInfo> {
    core.voice().stt_models()
}

/// Only ever called from an explicit Download click in Settings (rule 10).
#[tauri::command(rename_all = "snake_case")]
pub async fn download_stt_model(core: CoreState<'_>, id: String) -> Result<()> {
    core.voice().download_stt_model(&id).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_input_devices(core: CoreState<'_>) -> Vec<String> {
    core.voice().input_devices()
}

/// Register with `tauri::Builder::invoke_handler(tandem_core::commands::handler())`.
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        get_state,
        start_session,
        pause_session,
        resume_session,
        extend_session,
        end_session,
        close_session,
        start_break,
        add_subtasks,
        complete_subtask,
        set_quiet,
        set_checkin_mode,
        send_message,
        update_settings,
        update_profile,
        set_api_key,
        clear_api_key,
        refresh_provider,
        list_ollama_models,
        session_history,
        show_main_window,
        set_buddy_visible,
        voice_status,
        ptt_start,
        ptt_stop,
        ptt_cancel,
        stop_speaking,
        list_voices,
        download_voice,
        preview_voice,
        list_stt_models,
        download_stt_model,
        list_input_devices,
    ]
}
