//! Product CLI shell: the KARNA-style wordmark branding and the single
//! StatusSpinner line that replaces machinery output in normal mode.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const ORANGE: u8 = 208;
const WARM_RED: u8 = 202;
const RED: u8 = 196;

const WORDMARK: &str = r#"██╗  ██╗ █████╗ ██████╗ ██╗   ██╗ █████╗
██║ ██╔╝██╔══██╗██╔══██╗████╗  ██║██╔══██╗
█████╔╝ ███████║██████╔╝██╔██╗ ██║███████║
██╔═██╗ ██╔══██║██╔══██╗██║╚██╗██║██╔══██║
██║  ██╗██║  ██║██║  ██║██║ ╚████║██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝"#;

pub fn accent(style_color: u8, text: &str) -> String {
    if console::Term::stdout().features().colors_supported() {
        format!("\x1b[38;5;{style_color}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// The wordmark identity used by `kerna init` and `kerna code`; the orange to
/// warm-red gradient is the brand accent from the V1 plan.
pub fn banner(tagline: &str) {
    for (index, line) in WORDMARK.lines().enumerate() {
        let color = if index < 2 {
            ORANGE
        } else if index < 4 {
            WARM_RED
        } else {
            RED
        };
        println!("{}", accent(color, line));
    }
    println!();
    println!("            {}", accent(WARM_RED, tagline));
    println!();
}

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub struct StatusSpinner {
    stop: Arc<AtomicBool>,
    label: Arc<Mutex<String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl StatusSpinner {
    pub fn start(label: &str) -> Option<Self> {
        if !console::Term::stderr().is_term() {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let label = Arc::new(Mutex::new(label.to_string()));
        let (thread_stop, thread_label) = (Arc::clone(&stop), Arc::clone(&label));
        let worker = std::thread::spawn(move || {
            let term = console::Term::stderr();
            if term.hide_cursor().is_err() {
                return;
            }
            let mut frame = 0usize;
            while !thread_stop.load(Ordering::Relaxed) {
                let text = thread_label.lock().map(|l| l.clone()).unwrap_or_default();
                let _ = term.clear_line();
                let _ = term.write_str(&format!(
                    "{} {}...",
                    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()],
                    text
                ));
                let _ = term.flush();
                frame += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = term.clear_line();
            let _ = term.show_cursor();
            let _ = term.flush();
        });
        Some(Self {
            stop,
            label,
            worker: Some(worker),
        })
    }

    pub fn set_label(&self, label: &str) {
        if let Ok(mut current) = self.label.lock() {
            *current = label.to_string();
        }
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One active spinner per process, so governance prompts can borrow the
/// terminal back from the activity line without a refactor of the exec engine.
static ACTIVE: OnceLock<Mutex<Option<ActiveSpinner>>> = OnceLock::new();

struct ActiveSpinner {
    label: String,
    spinner: StatusSpinner,
}

fn active() -> &'static Mutex<Option<ActiveSpinner>> {
    ACTIVE.get_or_init(|| Mutex::new(None))
}

pub fn spinner_start(label: &str) {
    let mut slot = active().lock().expect("spinner lock poisoned");
    if let Some(existing) = slot.take() {
        existing.spinner.set_label(label);
        *slot = Some(existing);
        return;
    }
    if let Some(spinner) = StatusSpinner::start(label) {
        *slot = Some(ActiveSpinner {
            label: label.to_string(),
            spinner,
        });
    }
}

pub fn spinner_set_label(label: &str) {
    let mut guard = active().lock().expect("spinner lock poisoned");
    if let Some(existing) = guard.as_mut() {
        existing.label = label.to_string();
        existing.spinner.set_label(label);
    }
}

pub fn spinner_stop() {
    if let Some(existing) = active().lock().expect("spinner lock poisoned").take() {
        existing.spinner.stop();
    }
}

/// Runs a closure with the spinner paused, then resumes it with its last
/// label. Approval menus use this to own the terminal while a task works.
pub fn with_spinner_paused<T>(run: impl FnOnce() -> T) -> T {
    let saved = {
        let mut guard = active().lock().expect("spinner lock poisoned");
        guard.take().map(|existing| {
            existing.spinner.stop();
            existing.label
        })
    };
    let result = run();
    if let Some(label) = saved {
        spinner_start(&label);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_is_plain_and_stable() {
        assert_eq!(WORDMARK.lines().count(), 6);
        assert!(WORDMARK.contains('█'));
        assert!(!accent(ORANGE, "Kerna").contains('\n'));
    }

    #[test]
    fn paused_spinner_round_trips_through_a_borrowed_terminal() {
        // In a non-terminal test environment there is no spinner to pause;
        // the wrapper must still run the closure exactly once.
        let calls = std::cell::Cell::new(0u8);
        let value = with_spinner_paused(|| {
            calls.set(calls.get() + 1);
            7
        });
        assert_eq!(value, 7);
        assert_eq!(calls.get(), 1);
    }
}
