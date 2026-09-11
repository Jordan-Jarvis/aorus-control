use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use eframe::egui;
use ksni::blocking::{Handle, TrayMethods};

const SOCKET_NAME: &str = "aorus-control.sock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Open,
    Quit,
}

pub struct DesktopLifecycle {
    event_tx: Sender<Event>,
    event_rx: Receiver<Event>,
    _instance: InstanceGuard,
    repaint: Arc<Mutex<Option<egui::Context>>>,
}

impl DesktopLifecycle {
    pub fn claim(open_existing: bool) -> io::Result<Option<Self>> {
        let path = socket_path();
        if notify_existing(&path, open_existing) {
            return Ok(None);
        }

        if path.exists() {
            fs::remove_file(&path)?;
        }
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(_error) if notify_existing(&path, open_existing) => return Ok(None),
            Err(error) => return Err(error),
        };
        let (event_tx, event_rx) = mpsc::channel();
        let listener_tx = event_tx.clone();
        let repaint = Arc::new(Mutex::new(None::<egui::Context>));
        let listener_repaint = Arc::clone(&repaint);
        thread::Builder::new()
            .name("aorus-single-instance".into())
            .spawn(move || {
                for mut connection in listener.incoming().flatten() {
                    let mut command = String::new();
                    if connection.read_to_string(&mut command).is_ok()
                        && command.starts_with("open")
                    {
                        let _ = listener_tx.send(Event::Open);
                        if let Some(ctx) = listener_repaint.lock().ok().and_then(|ctx| ctx.clone())
                        {
                            ctx.request_repaint();
                        }
                    }
                }
            })?;

        Ok(Some(Self {
            event_tx,
            event_rx,
            _instance: InstanceGuard(path),
            repaint,
        }))
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.event_rx.try_recv().ok()
    }

    pub fn start_tray(&self, ctx: egui::Context) -> Result<TrayHandle, ksni::Error> {
        if let Ok(mut repaint) = self.repaint.lock() {
            *repaint = Some(ctx.clone());
        }
        let tray = AorusTray {
            event_tx: self.event_tx.clone(),
            ctx,
        };
        tray.assume_sni_available(true)
            .spawn()
            .map(|handle| TrayHandle { _handle: handle })
    }
}

pub struct TrayHandle {
    // Keeping the handle alive keeps the tray registration alive.
    _handle: Handle<AorusTray>,
}

struct AorusTray {
    event_tx: Sender<Event>,
    ctx: egui::Context,
}

impl AorusTray {
    fn send(&self, event: Event) {
        let _ = self.event_tx.send(event);
        self.ctx.request_repaint();
    }
}

impl ksni::Tray for AorusTray {
    fn id(&self) -> String {
        "io.github.aoruslinux.Control".into()
    }

    fn title(&self) -> String {
        "AORUS Control".into()
    }

    fn icon_name(&self) -> String {
        "io.github.aoruslinux.Control".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Event::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open AORUS Control".into(),
                icon_name: "window-new".into(),
                activate: Box::new(|tray: &mut AorusTray| tray.send(Event::Open)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut AorusTray| tray.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

struct InstanceGuard(PathBuf);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn notify_existing(path: &PathBuf, open_existing: bool) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    if open_existing {
        let _ = stream.write_all(b"open\n");
    }
    true
}

fn socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(SOCKET_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_name_is_stable() {
        assert_eq!(socket_path().file_name().unwrap(), SOCKET_NAME);
    }
}
