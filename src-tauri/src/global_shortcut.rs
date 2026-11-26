//! The global shortcut (Settings › Preferences): works from any app and opens the floating
//! command palette (see `palette_window.rs`) without bringing up the main window. Off until set.
//!
//! The setting is stored in the palette's canonical form (`Meta+Shift+B`, built by
//! `src/lib/palette.ts`); [`accelerator`] maps it to the plugin's syntax.
use sqlx::{Pool, Sqlite};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::AppError;

const MAX_SHORTCUT_LEN: usize = 64;

/// `Meta+Shift+B` → `Super+Shift+B`; the other tokens already match the plugin's names.
pub fn accelerator(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|t| if t == "Meta" { "Super" } else { t })
        .collect::<Vec<_>>()
        .join("+")
}

/// Parses a stored shortcut; it must include ⌘, ⌃ or ⌥ (a bare key would swallow typing).
pub fn parse(shortcut: &str) -> Result<Shortcut, AppError> {
    let shortcut = shortcut.trim();
    if shortcut.len() > MAX_SHORTCUT_LEN {
        return Err(AppError::Custom("Shortcut is too long".to_string()));
    }
    let tokens: Vec<&str> = shortcut.split('+').collect();
    if !tokens.iter().any(|t| matches!(*t, "Meta" | "Ctrl" | "Alt")) {
        return Err(AppError::Custom(
            "Include ⌘, ⌃ or ⌥ in the shortcut".to_string(),
        ));
    }
    accelerator(shortcut)
        .parse::<Shortcut>()
        .map_err(|_| AppError::Custom("Badger can't use that key as a global shortcut".to_string()))
}

/// The stored shortcut, or `None` when off (or unreadable).
pub async fn inner_get_global_shortcut(pool: &Pool<Sqlite>) -> Result<Option<String>, AppError> {
    Ok(crate::app_settings::get_global_shortcut(pool)
        .await?
        .filter(|s| parse(s).is_ok()))
}

/// Validates and stores the shortcut (`None` or empty turns it off). Returns what was stored.
pub async fn inner_set_global_shortcut(
    shortcut: Option<&str>,
    pool: &Pool<Sqlite>,
) -> Result<Option<String>, AppError> {
    let shortcut = shortcut.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = shortcut {
        parse(s)?;
    }
    crate::app_settings::set_global_shortcut(pool, shortcut.unwrap_or("")).await?;
    Ok(shortcut.map(String::from))
}

/// The plugin, with the handler every registered shortcut runs: toggle the floating palette.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                crate::palette_window::toggle(app);
            }
        })
        .build()
}

/// Registers `shortcut` as the only global shortcut (`None`: none).
pub fn register(app: &AppHandle, shortcut: Option<&str>) -> Result<(), AppError> {
    let gs = app.global_shortcut();
    gs.unregister_all()
        .map_err(|e| AppError::Custom(format!("Could not clear the global shortcut: {e}")))?;
    if let Some(s) = shortcut {
        gs.register(parse(s)?).map_err(|_| {
            AppError::Custom("That shortcut is already used by another app".to_string())
        })?;
        crate::palette_window::preload(app);
    }
    Ok(())
}

/// Registers the stored shortcut at launch. Never fails: problems are logged.
pub async fn install(app: &AppHandle, pool: &Pool<Sqlite>) {
    match inner_get_global_shortcut(pool).await {
        Ok(shortcut) => {
            if let Err(e) = register(app, shortcut.as_deref()) {
                tracing::error!("Global shortcut: not registered: {}", e);
            }
        }
        Err(e) => tracing::error!("Global shortcut: couldn't read the setting: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_and_validates_shortcuts() {
        assert_eq!(accelerator("Meta+Shift+B"), "Super+Shift+B");
        assert_eq!(accelerator("Ctrl+Alt+Space"), "Ctrl+Alt+Space");
        for ok in [
            "Meta+Shift+B",
            "Ctrl+Alt+Space",
            "Alt+1",
            "Meta+Comma",
            "Ctrl+F5",
        ] {
            assert!(parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            parse("Shift+B").unwrap_err().to_string(),
            "Include ⌘, ⌃ or ⌥ in the shortcut"
        );
        assert_eq!(
            parse("Meta+Shift+NotAKey").unwrap_err().to_string(),
            "Badger can't use that key as a global shortcut"
        );
    }

    #[tokio::test]
    async fn stores_and_clears_the_setting() {
        let pool = crate::test_db().await;
        assert_eq!(inner_get_global_shortcut(&pool).await.unwrap(), None);
        assert_eq!(
            inner_set_global_shortcut(Some(" Meta+Shift+B "), &pool)
                .await
                .unwrap()
                .as_deref(),
            Some("Meta+Shift+B")
        );
        assert_eq!(
            inner_get_global_shortcut(&pool).await.unwrap().as_deref(),
            Some("Meta+Shift+B")
        );
        assert!(inner_set_global_shortcut(Some("B"), &pool).await.is_err());
        assert_eq!(
            inner_get_global_shortcut(&pool).await.unwrap().as_deref(),
            Some("Meta+Shift+B"),
            "a refused shortcut keeps the old one"
        );
        assert_eq!(
            inner_set_global_shortcut(Some(""), &pool).await.unwrap(),
            None
        );
        assert_eq!(inner_get_global_shortcut(&pool).await.unwrap(), None);
    }
}
