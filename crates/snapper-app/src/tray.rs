//! Tray icon and the actions the menu and hotkeys share.
//!
//! macOS menu-bar mode (`LSUIElement`, no Dock icon) is applied by packaging
//! from `macos/Info.plist`. `cargo run` is not an app bundle, so the Dock
//! icon remains until that bundle exists.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// What a tray item or hotkey asks the app to do.
///
/// [`Action::CaptureLastRegion`] is a hotkey only. It is not a tray item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CaptureRegion,
    CaptureWindow,
    CaptureFull,
    CaptureLastRegion,
    Record,
    Ocr,
    ScrollCapture,
    /// Index into [`snapper_core::Settings::workflows`].
    Workflow(u16),
    OpenFolder,
    Settings,
    Quit,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::CaptureRegion => "Capture region",
            Self::CaptureWindow => "Capture window",
            Self::CaptureFull => "Capture full screen",
            Self::CaptureLastRegion => "Capture last region",
            Self::Record => "Record",
            Self::Ocr => "OCR region",
            Self::ScrollCapture => "Scrolling capture",
            Self::Workflow(_) => "Workflow",
            Self::OpenFolder => "Open folder",
            Self::Settings => "Settings",
            Self::Quit => "Quit",
        }
    }

    fn menu_id(self) -> Option<&'static str> {
        Some(match self {
            Self::CaptureRegion => "capture-region",
            Self::CaptureWindow => "capture-window",
            Self::CaptureFull => "capture-full",
            Self::CaptureLastRegion => return None,
            Self::Record => "record",
            Self::Ocr => "ocr",
            Self::ScrollCapture => "scroll-capture",
            Self::Workflow(_) => return None,
            Self::OpenFolder => "open-folder",
            Self::Settings => "settings",
            Self::Quit => "quit",
        })
    }
}

const MENU_ACTIONS: [Action; 9] = [
    Action::CaptureRegion,
    Action::CaptureWindow,
    Action::CaptureFull,
    Action::Record,
    Action::Ocr,
    Action::ScrollCapture,
    Action::OpenFolder,
    Action::Settings,
    Action::Quit,
];

pub fn action_from_id(id: &str) -> Option<Action> {
    if let Some(rest) = id.strip_prefix("workflow-") {
        let index = rest.parse().ok()?;
        return Some(Action::Workflow(index));
    }
    MENU_ACTIONS
        .into_iter()
        .find(|action| action.menu_id() == Some(id))
}

/// Build the tray icon. Call this on the thread that runs the event loop.
pub fn build(settings: &snapper_core::Settings) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    for action in MENU_ACTIONS {
        let id = action.menu_id().expect("tray action has an id");
        let item = MenuItem::with_id(id, action.label(), true, None);
        menu.append(&item).map_err(|err| err.to_string())?;
    }
    for (index, workflow) in settings.workflows.iter().enumerate() {
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        let item = MenuItem::with_id(format!("workflow-{index}"), &workflow.name, true, None);
        menu.append(&item).map_err(|err| err.to_string())?;
    }
    let icon = Icon::from_rgba(icon_rgba(), 32, 32).map_err(|err| err.to_string())?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Snapper")
        .with_icon(icon)
        .with_icon_as_template(true)
        .build()
        .map_err(|err| err.to_string())
}

pub fn poll_menu() -> Option<Action> {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        let id = menu_event_id(&event);
        if let Some(action) = action_from_id(id) {
            return Some(action);
        }
    }
    None
}

fn menu_event_id(event: &MenuEvent) -> &str {
    event.id.as_ref()
}

/// 32×32 tray mark: a blue disc with a white centre.
fn icon_rgba() -> Vec<u8> {
    const N: u32 = 32;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    let centre = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let distance = (dx * dx + dy * dy).sqrt();
            let index = ((y * N + x) * 4) as usize;
            if distance <= 6.0 {
                rgba[index] = 255;
                rgba[index + 1] = 255;
                rgba[index + 2] = 255;
                rgba[index + 3] = 255;
            } else if distance <= 14.0 {
                rgba[index] = 0x1F;
                rgba[index + 1] = 0x6F;
                rgba[index + 2] = 0xE5;
                rgba[index + 3] = 255;
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip_except_last_region() {
        for action in MENU_ACTIONS {
            let id = action.menu_id().expect("menu id");
            assert_eq!(action_from_id(id), Some(action));
        }
        assert_eq!(Action::CaptureLastRegion.menu_id(), None);
        assert_eq!(action_from_id("capture-last-region"), None);
        assert_eq!(action_from_id(""), None);
    }

    #[test]
    fn icon_is_square_rgba() {
        let rgba = icon_rgba();
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert!(rgba.iter().any(|byte| *byte != 0));
    }
}
