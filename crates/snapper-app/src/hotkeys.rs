//! Parse settings chords and register them with `global-hotkey`.
//!
//! The settings strings (`PrintScreen`, `Ctrl+Shift+R`) are not the chord
//! syntax `global-hotkey` parses itself (`control+shift+KeyR`). [`parse`]
//! accepts the settings form.
//!
//! On Wayland, `global-hotkey` 0.6 can only grab keys through X11. The
//! GlobalShortcuts portal is out of scope; see `docs/platforms/linux-wayland.md`.
//! A chord the OS rejects is returned as a conflict so the app can notify.

use std::collections::HashMap;
use std::fmt;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use snapper_core::Settings;

use crate::tray::Action;

/// A settings chord that is not a modifier plus one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyParseError {
    chord: String,
    reason: String,
}

impl fmt::Display for HotkeyParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not parse hotkey \"{}\": {}",
            self.chord, self.reason
        )
    }
}

impl std::error::Error for HotkeyParseError {}

impl HotkeyParseError {
    fn new(chord: &str, reason: impl Into<String>) -> Self {
        Self {
            chord: chord.to_owned(),
            reason: reason.into(),
        }
    }
}

/// Parse `Ctrl+Shift+PrintScreen` into a [`HotKey`].
///
/// Modifiers may appear in any order. `Ctrl`/`Control`, `Alt`/`Option`,
/// `Shift`, and `Super`/`Meta`/`Win`/`Cmd` are accepted. The key is
/// case-insensitive (`r`, `KeyR`, `PrintScreen`).
pub fn parse(chord: &str) -> Result<HotKey, HotkeyParseError> {
    let trimmed = chord.trim();
    if trimmed.is_empty() {
        return Err(HotkeyParseError::new(chord, "chord is empty"));
    }

    let mut mods = Modifiers::empty();
    let mut key = None;
    for raw in trimmed.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            return Err(HotkeyParseError::new(chord, "empty key name"));
        }
        if let Some(modifier) = modifier(token) {
            mods |= modifier;
            continue;
        }
        if key.is_some() {
            return Err(HotkeyParseError::new(
                chord,
                format!("more than one key in \"{token}\""),
            ));
        }
        key = Some(
            parse_key(token)
                .ok_or_else(|| HotkeyParseError::new(chord, format!("unknown key \"{token}\"")))?,
        );
    }

    let Some(key) = key else {
        return Err(HotkeyParseError::new(chord, "chord has no key"));
    };
    let mods = if mods.is_empty() { None } else { Some(mods) };
    Ok(HotKey::new(mods, key))
}

/// Canonical settings chord, such as `Ctrl+Shift+R` or `PrintScreen`.
///
/// Modifier order is Ctrl, Alt, Shift, then Super. The result is accepted by
/// [`parse`] for every key [`parse`] itself accepts. Any other key is `Unknown`.
pub fn format(hotkey: &HotKey) -> String {
    let mut parts = Vec::new();
    if hotkey.mods.contains(Modifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if hotkey.mods.contains(Modifiers::ALT) {
        parts.push("Alt");
    }
    if hotkey.mods.contains(Modifiers::SHIFT) {
        parts.push("Shift");
    }
    if hotkey.mods.contains(Modifiers::SUPER) {
        parts.push("Super");
    }
    parts.push(format_code(hotkey.key));
    parts.join("+")
}

fn format_code(key: Code) -> &'static str {
    match key {
        Code::PrintScreen => "PrintScreen",
        Code::Escape => "Escape",
        Code::Space => "Space",
        Code::Tab => "Tab",
        Code::Enter => "Enter",
        Code::Backspace => "Backspace",
        Code::Delete => "Delete",
        Code::Insert => "Insert",
        Code::ArrowUp => "Up",
        Code::ArrowDown => "Down",
        Code::ArrowLeft => "Left",
        Code::ArrowRight => "Right",
        Code::PageUp => "PageUp",
        Code::PageDown => "PageDown",
        Code::Home => "Home",
        Code::End => "End",
        Code::F1 => "F1",
        Code::F2 => "F2",
        Code::F3 => "F3",
        Code::F4 => "F4",
        Code::F5 => "F5",
        Code::F6 => "F6",
        Code::F7 => "F7",
        Code::F8 => "F8",
        Code::F9 => "F9",
        Code::F10 => "F10",
        Code::F11 => "F11",
        Code::F12 => "F12",
        Code::KeyA => "A",
        Code::KeyB => "B",
        Code::KeyC => "C",
        Code::KeyD => "D",
        Code::KeyE => "E",
        Code::KeyF => "F",
        Code::KeyG => "G",
        Code::KeyH => "H",
        Code::KeyI => "I",
        Code::KeyJ => "J",
        Code::KeyK => "K",
        Code::KeyL => "L",
        Code::KeyM => "M",
        Code::KeyN => "N",
        Code::KeyO => "O",
        Code::KeyP => "P",
        Code::KeyQ => "Q",
        Code::KeyR => "R",
        Code::KeyS => "S",
        Code::KeyT => "T",
        Code::KeyU => "U",
        Code::KeyV => "V",
        Code::KeyW => "W",
        Code::KeyX => "X",
        Code::KeyY => "Y",
        Code::KeyZ => "Z",
        Code::Digit0 => "0",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        _ => "Unknown",
    }
}

/// Registrations for one process. Dropping the service unregisters them.
pub struct HotkeyService {
    /// Kept so the operating system registrations stay alive.
    _manager: GlobalHotKeyManager,
    by_id: HashMap<u32, Action>,
}

impl HotkeyService {
    /// Register every chord in `hotkeys`.
    ///
    /// Parse failures, two actions that share a chord, and chords the OS
    /// refuses are conflicts. The `String` names the chord and the reason.
    /// Only a failure to create the manager is an error.
    pub fn register(settings: &Settings) -> Result<(Self, Vec<(Action, String)>), String> {
        let manager = GlobalHotKeyManager::new().map_err(|err| err.to_string())?;
        let (bindings, mut conflicts) = plan(settings);
        let mut by_id = HashMap::new();
        for binding in bindings {
            match manager.register(binding.hotkey) {
                Ok(()) => {
                    by_id.insert(binding.hotkey.id(), binding.action);
                }
                Err(err) => conflicts.push((
                    binding.action,
                    format!("{} could not be registered: {err}", binding.chord),
                )),
            }
        }
        Ok((
            Self {
                _manager: manager,
                by_id,
            },
            conflicts,
        ))
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        self.by_id.get(&id).copied()
    }
}

struct Binding {
    action: Action,
    hotkey: HotKey,
    chord: String,
}

fn plan(settings: &Settings) -> (Vec<Binding>, Vec<(Action, String)>) {
    let mut bindings = Vec::new();
    let mut conflicts = Vec::new();
    let mut seen: Vec<(u32, Action, String)> = Vec::new();
    for (action, chord) in chords(settings) {
        let chord = chord.trim();
        if chord.is_empty() {
            conflicts.push((action, "hotkey is empty".to_owned()));
            continue;
        }
        let hotkey = match parse(chord) {
            Ok(hotkey) => hotkey,
            Err(err) => {
                conflicts.push((action, err.to_string()));
                continue;
            }
        };
        if let Some((_, other, other_chord)) = seen.iter().find(|(id, _, _)| *id == hotkey.id()) {
            conflicts.push((
                action,
                format!(
                    "{chord} is already used by {} ({other_chord})",
                    other.label()
                ),
            ));
            continue;
        }
        seen.push((hotkey.id(), action, chord.to_owned()));
        bindings.push(Binding {
            action,
            hotkey,
            chord: chord.to_owned(),
        });
    }
    (bindings, conflicts)
}

fn chords(settings: &Settings) -> Vec<(Action, String)> {
    let hotkeys = &settings.hotkeys;
    let mut chords = vec![
        (Action::CaptureRegion, hotkeys.region.clone()),
        (Action::CaptureWindow, hotkeys.window.clone()),
        (Action::CaptureFull, hotkeys.full_screen.clone()),
        (Action::CaptureLastRegion, hotkeys.last_region.clone()),
        (Action::Record, hotkeys.record.clone()),
        (Action::Ocr, hotkeys.ocr.clone()),
        (Action::ScrollCapture, hotkeys.scroll.clone()),
    ];
    for (index, workflow) in settings.workflows.iter().enumerate() {
        if workflow.hotkey.trim().is_empty() {
            continue;
        }
        let Ok(index) = u16::try_from(index) else {
            continue;
        };
        chords.push((Action::Workflow(index), workflow.hotkey.clone()));
    }
    chords
}

/// Drop queued press and release events.
///
/// Re-registering replaces the manager, but a chord pressed while the settings
/// window was open would still be sitting in the process-wide queue. The shell
/// would treat it as a new capture as soon as it started again.
pub fn drain_events() {
    while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
}

/// The next pressed hotkey, if one is waiting.
pub fn poll_pressed(service: &HotkeyService) -> Option<Action> {
    while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
        if event.state != HotKeyState::Pressed {
            continue;
        }
        if let Some(action) = service.action(event.id) {
            return Some(action);
        }
    }
    None
}

fn modifier(token: &str) -> Option<Modifiers> {
    Some(match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Modifiers::CONTROL,
        "alt" | "option" => Modifiers::ALT,
        "shift" => Modifiers::SHIFT,
        "super" | "meta" | "win" | "windows" | "cmd" | "command" => Modifiers::SUPER,
        _ => return None,
    })
}

fn parse_key(token: &str) -> Option<Code> {
    let lower = token.to_ascii_lowercase();
    if let Some(key) = named_key(&lower) {
        return Some(key);
    }
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(number) = rest.parse::<u8>() {
            return function_key(number);
        }
    }
    if lower.len() == 1 {
        let ch = lower.chars().next()?;
        if ch.is_ascii_digit() {
            return digit(ch);
        }
        if ch.is_ascii_alphabetic() {
            return letter(ch.to_ascii_uppercase());
        }
    }
    if let Some(rest) = token.strip_prefix("Key") {
        if rest.len() == 1 {
            return letter(rest.chars().next()?.to_ascii_uppercase());
        }
    }
    if let Some(rest) = token.strip_prefix("Digit") {
        if rest.len() == 1 {
            return digit(rest.chars().next()?);
        }
    }
    None
}

fn named_key(lower: &str) -> Option<Code> {
    Some(match lower {
        "printscreen" | "print" | "prtsc" | "prtscr" | "snapshot" => Code::PrintScreen,
        "escape" | "esc" => Code::Escape,
        "space" => Code::Space,
        "tab" => Code::Tab,
        "enter" | "return" => Code::Enter,
        "backspace" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "insert" | "ins" => Code::Insert,
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "home" => Code::Home,
        "end" => Code::End,
        _ => return None,
    })
}

fn function_key(number: u8) -> Option<Code> {
    Some(match number {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        _ => return None,
    })
}

fn letter(ch: char) -> Option<Code> {
    Some(match ch {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return None,
    })
}

fn digit(ch: char) -> Option<Code> {
    Some(match ch {
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapper_core::settings::HotkeySettings;

    fn assert_chord(chord: &str, mods: Option<Modifiers>, key: Code) {
        let parsed = parse(chord).unwrap_or_else(|err| panic!("{chord}: {err}"));
        let expected = HotKey::new(mods, key);
        assert_eq!(parsed.mods, expected.mods, "{chord}");
        assert_eq!(parsed.key, expected.key, "{chord}");
        assert_eq!(parsed.id(), expected.id(), "{chord}");
    }

    #[test]
    fn default_chords_parse() {
        assert_chord("PrintScreen", None, Code::PrintScreen);
        assert_chord("Alt+PrintScreen", Some(Modifiers::ALT), Code::PrintScreen);
        assert_chord(
            "Ctrl+PrintScreen",
            Some(Modifiers::CONTROL),
            Code::PrintScreen,
        );
        assert_chord(
            "Shift+PrintScreen",
            Some(Modifiers::SHIFT),
            Code::PrintScreen,
        );
        assert_chord(
            "Ctrl+Shift+R",
            Some(Modifiers::CONTROL | Modifiers::SHIFT),
            Code::KeyR,
        );
    }

    #[test]
    fn modifiers_accept_aliases_and_any_order() {
        assert_chord(
            "Ctrl+Shift+PrintScreen",
            Some(Modifiers::CONTROL | Modifiers::SHIFT),
            Code::PrintScreen,
        );
        assert_chord(
            "printscreen+meta",
            Some(Modifiers::SUPER),
            Code::PrintScreen,
        );
        assert_chord(
            " cmd + option + KeyP ",
            Some(Modifiers::SUPER | Modifiers::ALT),
            Code::KeyP,
        );
        assert_chord(
            "control+shift+r",
            Some(Modifiers::CONTROL | Modifiers::SHIFT),
            Code::KeyR,
        );
        assert_chord("F1", None, Code::F1);
        assert_chord("Digit5", None, Code::Digit5);
    }

    #[test]
    fn format_canonicalises_modifier_order_and_key_names() {
        let cases = [
            ("PrintScreen", "PrintScreen"),
            ("Alt+PrintScreen", "Alt+PrintScreen"),
            ("shift+ctrl+r", "Ctrl+Shift+R"),
            (" cmd + option + KeyP ", "Alt+Super+P"),
            ("f12", "F12"),
            ("Digit5", "5"),
            ("up", "Up"),
        ];
        for (input, expected) in cases {
            let hotkey = parse(input).unwrap_or_else(|err| panic!("{input}: {err}"));
            let shown = format(&hotkey);
            assert_eq!(shown, expected, "{input}");
            let again = parse(&shown).unwrap_or_else(|err| panic!("{shown}: {err}"));
            assert_eq!(again.id(), hotkey.id(), "{input}");
        }
    }

    #[test]
    fn empty_unknown_and_modifier_only_chords_fail() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+Shift").is_err());
        assert!(parse("NotAKey")
            .unwrap_err()
            .to_string()
            .contains("NotAKey"));
        assert!(parse("Ctrl+Shift+R+T").is_err());
    }

    #[test]
    fn default_settings_have_no_parse_conflicts() {
        let (bindings, conflicts) = plan(&Settings::default());
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(bindings.len(), 7);
        assert!(bindings.iter().any(|binding| binding.action == Action::Ocr));
        assert!(bindings
            .iter()
            .any(|binding| binding.action == Action::ScrollCapture));
    }

    #[test]
    fn an_empty_workflow_hotkey_is_skipped() {
        let mut settings = Settings::default();
        settings.workflows.push(snapper_core::settings::Workflow {
            name: "Bug report".to_owned(),
            hotkey: "  ".to_owned(),
            capture: snapper_core::settings::CaptureKind::Region,
            steps: Vec::new(),
            link_format: snapper_core::settings::LinkFormat::Markdown,
        });
        settings.workflows.push(snapper_core::settings::Workflow {
            name: "Upload".to_owned(),
            hotkey: "Ctrl+Shift+U".to_owned(),
            capture: snapper_core::settings::CaptureKind::Region,
            steps: Vec::new(),
            link_format: snapper_core::settings::LinkFormat::Markdown,
        });
        let (bindings, conflicts) = plan(&settings);
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert!(bindings
            .iter()
            .any(|binding| binding.action == Action::Workflow(1)));
        assert!(!bindings
            .iter()
            .any(|binding| matches!(binding.action, Action::Workflow(0))));
    }

    #[test]
    fn duplicate_chords_are_conflicts() {
        let defaults = HotkeySettings::default();
        let hotkeys = HotkeySettings {
            window: defaults.region.clone(),
            ..defaults
        };
        let settings = Settings {
            hotkeys,
            ..Settings::default()
        };
        let (bindings, conflicts) = plan(&settings);
        assert!(bindings
            .iter()
            .any(|binding| binding.action == Action::CaptureRegion));
        assert!(!bindings
            .iter()
            .any(|binding| binding.action == Action::CaptureWindow));
        let (_, detail) = conflicts
            .iter()
            .find(|(action, _)| *action == Action::CaptureWindow)
            .expect("window conflict");
        assert!(detail.contains("Capture region"), "{detail}");
        assert!(detail.contains("PrintScreen"), "{detail}");
    }

    #[test]
    fn a_bad_chord_is_reported_as_a_conflict() {
        let hotkeys = HotkeySettings {
            record: "Nope".to_owned(),
            ..HotkeySettings::default()
        };
        let settings = Settings {
            hotkeys,
            ..Settings::default()
        };
        let (bindings, conflicts) = plan(&settings);
        assert!(!bindings
            .iter()
            .any(|binding| binding.action == Action::Record));
        let (_, detail) = conflicts
            .iter()
            .find(|(action, _)| *action == Action::Record)
            .expect("record conflict");
        assert!(detail.contains("Nope"), "{detail}");
    }
}
