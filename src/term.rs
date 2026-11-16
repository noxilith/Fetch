//! Terminal handling: raw mode + mouse reporting, size, signals, and input
//! (mouse drags rotate the logo; any other key ends the animation).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static TERM_RESIZED: AtomicBool = AtomicBool::new(false);
static TERM_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_winch(_: libc::c_int) {
    TERM_RESIZED.store(true, Ordering::Relaxed);
}

extern "C" fn on_stop(_: libc::c_int) {
    TERM_STOP.store(true, Ordering::Relaxed);
}

/// Install SIGINT/SIGTERM (stop) and SIGWINCH (resize) handlers. The handlers
/// only set flags; the animation loop acts on them.
pub fn install_signal_handlers() {
    // SAFETY: the handlers are async-signal-safe (they only touch atomics).
    unsafe {
        libc::signal(libc::SIGINT, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGWINCH, on_winch as *const () as libc::sighandler_t);
    }
}

pub fn stop_requested() -> bool {
    TERM_STOP.load(Ordering::Relaxed)
}

/// True once per resize.
pub fn take_resized() -> bool {
    TERM_RESIZED.swap(false, Ordering::Relaxed)
}

/// (rows, cols) of the terminal on stdout, 0 where unknown.
pub fn size() -> (i32, i32) {
    // SAFETY: winsize is plain data; ioctl fills it on success.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 {
            (i32::from(ws.ws_row), i32::from(ws.ws_col))
        } else {
            (0, 0)
        }
    }
}

/// Puts stdin in non-blocking raw-ish mode and restores everything on drop,
/// including on early exit and panic.
pub struct Terminal {
    saved: Option<libc::termios>,
}

impl Terminal {
    /// Registers the guard first (so the cleanup sequences are always printed),
    /// then switches stdin to non-canonical, no-echo, non-blocking reads.
    /// `hide_cursor` is false when the caller parks a visible cursor itself.
    pub fn enter(hide_cursor: bool) -> Terminal {
        let mut term = Terminal { saved: None };
        // SAFETY: termios is plain data; tcgetattr/tcsetattr only use it.
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut orig) == 0 {
                term.saved = Some(orig);
                let mut raw = orig;
                raw.c_lflag &= !(libc::ICANON | libc::ECHO);
                raw.c_cc[libc::VMIN] = 0;
                raw.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw);
            }
        }
        let mut out = std::io::stdout().lock();
        if hide_cursor {
            let _ = out.write_all(b"\x1b[?25l");
        }
        let _ = out.write_all(b"\x1b[?1002h\x1b[?1006h\x1b[2J");
        let _ = out.flush();
        term
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(orig) = &self.saved {
            // SAFETY: restoring the attributes we read in `enter`.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, orig) };
        }
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(b"\x1b[?1002l\x1b[?1006l\x1b[?25h");
        let _ = out.flush();
    }
}

/// Mouse-drag state plus the carry-over input buffer.
#[derive(Default)]
pub struct Input {
    buf: Vec<u8>,
    pub dragging: bool,
    last_x: i32,
    last_y: i32,
    /// Angular velocity left over from the last drag (the "fling").
    pub drag_vx: f32,
    pub drag_vy: f32,
}

fn digits(buf: &[u8], j: &mut usize) -> i32 {
    let mut n = 0i32;
    while *j < buf.len() && buf[*j].is_ascii_digit() {
        n = n.saturating_mul(10).saturating_add(i32::from(buf[*j] - b'0'));
        *j += 1;
    }
    n
}

fn stdin_pending() -> i32 {
    let mut avail: libc::c_int = 0;
    // SAFETY: FIONREAD writes one int.
    unsafe { libc::ioctl(libc::STDIN_FILENO, libc::FIONREAD, &mut avail) };
    avail
}

fn stdin_readable() -> bool {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut pfd, 1, 0) > 0 }
}

/// Hand bytes back to the terminal's input queue (as if typed again) so the
/// shell sees input that fetch read while deciding whether it was a mouse
/// event. Needs TIOCSTI, which Linux can disable (`dev.tty.legacy_tiocsti=0`);
/// without it the bytes are lost, as they would be for any program that reads them.
#[cfg(target_os = "linux")]
fn push_back(bytes: &[u8]) {
    for b in bytes {
        // SAFETY: TIOCSTI takes a pointer to one byte.
        let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCSTI as _, b as *const u8) };
        if rc != 0 {
            return;
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn push_back(_bytes: &[u8]) {}

fn stdin_read(dst: &mut [u8]) -> isize {
    // SAFETY: dst is valid for dst.len() bytes.
    unsafe { libc::read(libc::STDIN_FILENO, dst.as_mut_ptr().cast(), dst.len()) }
}

impl Input {
    /// Drain pending input. Mouse events drive `angle_a`/`angle_b` (drag to
    /// rotate); returns true if a regular key was pressed. A lone pending
    /// byte is left unread so the shell still receives that keypress.
    pub fn poll(&mut self, angle_a: &mut f32, angle_b: &mut f32) -> bool {
        const CAP: usize = 128;
        while stdin_readable() {
            if self.buf.is_empty() {
                // Mouse escapes are at least 9 bytes (ESC[<0;1;1M); a single
                // pending byte is an ordinary keypress.
                let avail = stdin_pending();
                if avail <= 0 {
                    break;
                }
                if avail == 1 {
                    return true;
                }
                let mut tmp = [0u8; CAP];
                let want = (avail as usize).min(CAP);
                let n = stdin_read(&mut tmp[..want]);
                if n <= 0 {
                    return true;
                }
                self.buf.extend_from_slice(&tmp[..n as usize]);
                if self.buf[0] != 0x1b {
                    self.give_back();
                    return true;
                }
            } else {
                let mut tmp = [0u8; CAP];
                let room = CAP.saturating_sub(self.buf.len());
                if room > 0 {
                    let n = stdin_read(&mut tmp[..room]);
                    if n > 0 {
                        self.buf.extend_from_slice(&tmp[..n as usize]);
                    }
                }
            }

            let (consumed, key) = self.parse(angle_a, angle_b);
            if key {
                self.buf.drain(..consumed.min(self.buf.len()));
                self.give_back();
                return true;
            }
            if consumed > 0 && consumed < self.buf.len() {
                self.buf.drain(..consumed);
            } else if consumed >= self.buf.len() {
                self.buf.clear();
            }
        }
        false
    }

    /// Return everything read but not used (plus anything that has arrived
    /// since, to keep the order) to the terminal for the shell to read.
    fn give_back(&mut self) {
        loop {
            let avail = stdin_pending();
            if avail <= 0 {
                break;
            }
            let mut tmp = [0u8; 128];
            let n = stdin_read(&mut tmp[..(avail as usize).min(128)]);
            if n <= 0 {
                break;
            }
            self.buf.extend_from_slice(&tmp[..n as usize]);
        }
        push_back(&self.buf);
        self.buf.clear();
    }

    /// Consume complete escape sequences from the buffer; returns bytes
    /// consumed and whether a non-escape key was seen.
    fn parse(&mut self, angle_a: &mut f32, angle_b: &mut f32) -> (usize, bool) {
        let len = self.buf.len();
        let mut i = 0;
        while i < len {
            if self.buf[i] != 0x1b {
                return (i, true);
            }
            if i + 2 >= len {
                break;
            }
            if self.buf[i + 1] == b'[' && self.buf[i + 2] == b'<' {
                let mut j = i + 3;
                let btn = digits(&self.buf, &mut j);
                if j >= len {
                    break;
                }
                if self.buf[j] == b';' {
                    j += 1;
                }
                let mx = digits(&self.buf, &mut j);
                if j >= len {
                    break;
                }
                if self.buf[j] == b';' {
                    j += 1;
                }
                let my = digits(&self.buf, &mut j);
                if j >= len {
                    break;
                }
                let trail = self.buf[j];
                j += 1;
                if trail != b'M' && trail != b'm' {
                    i = j;
                    continue;
                }

                if btn == 0 && trail == b'M' {
                    self.dragging = true;
                    self.last_x = mx;
                    self.last_y = my;
                    self.drag_vx = 0.0;
                    self.drag_vy = 0.0;
                } else if btn == 32 && trail == b'M' && self.dragging {
                    let dx = mx - self.last_x;
                    let dy = my - self.last_y;
                    self.drag_vy = -dx as f32 * 0.03;
                    self.drag_vx = -dy as f32 * 0.03;
                    *angle_b += self.drag_vy;
                    *angle_a += self.drag_vx;
                    self.last_x = mx;
                    self.last_y = my;
                } else if btn == 0 && trail == b'm' {
                    self.dragging = false;
                }
                i = j;
            } else {
                // Some other escape sequence (arrow keys etc.): a keypress,
                // handed back to the shell whole.
                return (i, true);
            }
        }
        (i, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(input: &mut Input, bytes: &[u8]) -> (usize, bool, f32, f32) {
        input.buf = bytes.to_vec();
        let (mut a, mut b) = (0.0, 0.0);
        let (n, key) = input.parse(&mut a, &mut b);
        (n, key, a, b)
    }

    #[test]
    fn mouse_drag_rotates() {
        let mut inp = Input::default();
        let (n, key, ..) = feed(&mut inp, b"\x1b[<0;10;10M");
        assert_eq!((n, key), (11, false));
        assert!(inp.dragging);
        let (_, key, a, b) = feed(&mut inp, b"\x1b[<32;12;13M");
        assert!(!key);
        assert!((b - -0.06).abs() < 1e-6 && (a - -0.09).abs() < 1e-6);
        feed(&mut inp, b"\x1b[<0;12;13m");
        assert!(!inp.dragging);
    }

    #[test]
    fn other_input_is_a_keypress() {
        let mut inp = Input::default();
        assert!(feed(&mut inp, b"q").1);
        assert!(feed(&mut inp, b"\x1b[A").1); // arrow keys end it too
    }

    #[test]
    fn incomplete_escape_waits() {
        let mut inp = Input::default();
        let (n, key, ..) = feed(&mut inp, b"\x1b[<0;1");
        assert_eq!((n, key), (0, false));
    }
}
