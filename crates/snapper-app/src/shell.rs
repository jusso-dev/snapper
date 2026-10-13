//! Hidden eframe window that routes tray and hotkey events.
//!
//! The tray icon and the hotkey manager are created on the event-loop thread
//! the first time the shell starts, then kept for the process. Dropping them
//! when the shell closes would unregister hotkeys and remove the icon while
//! the overlay or the editor is open. The shell itself closes and returns a
//! [`Command`]; the caller runs that work and starts the shell again.

use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex};

use egui::{ViewportBuilder, ViewportCommand};
use snapper_core::settings::HotkeySettings;

use crate::dispatch::{command_from_args, Command};
use crate::hotkeys::{self, HotkeyService};
use crate::output;
use crate::tray;

struct Resident {
    /// The icon stays in the tray until process exit.
    _tray: tray_icon::TrayIcon,
    hotkeys: HotkeyService,
}

thread_local! {
    static RESIDENT: RefCell<Option<Resident>> = const { RefCell::new(None) };
}

/// Run until the user picks an action, or the hidden window is closed.
///
/// `Ok(None)` means the window closed without an action (treat as quit).
pub fn run(
    hotkeys: &HotkeySettings,
    ipc: &Arc<Mutex<mpsc::Receiver<Vec<String>>>>,
) -> Result<Option<Command>, String> {
    let (tx, rx) = mpsc::channel();
    let ipc = Arc::clone(ipc);
    let hotkeys = hotkeys.clone();
    let viewport = ViewportBuilder::default()
        .with_title("Snapper")
        .with_inner_size([1.0, 1.0])
        .with_visible(false)
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false);
    let options = eframe::NativeOptions {
        viewport,
        persist_window: false,
        run_and_return: true,
        ..Default::default()
    };
    let created = eframe::run_native(
        "Snapper",
        options,
        Box::new(move |_cc| {
            install(&hotkeys)?;
            Ok(Box::new(Shell {
                ipc,
                outcome: tx,
                sent: false,
            }))
        }),
    );
    if let Err(err) = created {
        return Err(err.to_string());
    }
    Ok(rx.try_recv().ok())
}

fn install(hotkeys: &HotkeySettings) -> Result<(), String> {
    let already = RESIDENT.with(|slot| slot.borrow().is_some());
    if already {
        return Ok(());
    }
    let (service, conflicts) = HotkeyService::register(hotkeys)?;
    for (action, detail) in &conflicts {
        output::notify(
            "Snapper",
            &format!("Hotkey conflict: {}: {detail}", action.label()),
        );
    }
    let tray = tray::build()?;
    RESIDENT.with(|slot| {
        *slot.borrow_mut() = Some(Resident {
            _tray: tray,
            hotkeys: service,
        });
    });
    Ok(())
}

struct Shell {
    ipc: Arc<Mutex<mpsc::Receiver<Vec<String>>>>,
    outcome: mpsc::Sender<Command>,
    sent: bool,
}

impl Shell {
    fn trigger(&mut self, command: Command, ctx: &egui::Context) {
        if self.sent {
            return;
        }
        self.sent = true;
        let _ = self.outcome.send(command);
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

impl eframe::App for Shell {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let forwarded = self.ipc.lock().ok().and_then(|incoming| {
            let mut found = None;
            while let Ok(args) = incoming.try_recv() {
                if let Some(command) = command_from_args(&args) {
                    found = Some(command);
                    break;
                }
            }
            found
        });
        if let Some(command) = forwarded {
            self.trigger(command, ctx);
            return;
        }

        if let Some(action) = tray::poll_menu() {
            self.trigger(Command::Action(action), ctx);
            return;
        }
        let pressed = RESIDENT.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|resident| hotkeys::poll_pressed(&resident.hotkeys))
        });
        if let Some(action) = pressed {
            self.trigger(Command::Action(action), ctx);
            return;
        }

        if !self.sent {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
