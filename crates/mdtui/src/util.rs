//! Clipboard (OSC 52) and ANSI output for `--dump`.

use std::io::Write;

use base64::Engine;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

fn pipe_to(cmd: &str, args: &[&str], text: &str) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let ok = child
        .stdin
        .take()
        .map(|mut s| s.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    // xclip / wl-copy fork to keep serving the selection; the parent exits right away
    ok && child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Copy text to the clipboard. Tries, in order: tmux's buffer (forwarded to the outer
/// terminal), the desktop clipboard (wl-copy, xclip, xsel, pbcopy, clip.exe), and always
/// also emits OSC 52 so it works over SSH in terminals that support it.
/// Returns a short name of what worked.
pub fn copy_to_clipboard(text: &str) -> &'static str {
    let mut method = "";
    if std::env::var_os("TMUX").is_some() && pipe_to("tmux", &["load-buffer", "-w", "-"], text) {
        method = "tmux";
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    let system = if wayland && pipe_to("wl-copy", &[], text) {
        "wl-copy"
    } else if x11 && pipe_to("xclip", &["-selection", "clipboard", "-in"], text) {
        "xclip"
    } else if x11 && pipe_to("xsel", &["--clipboard", "--input"], text) {
        "xsel"
    } else if cfg!(target_os = "macos") && pipe_to("pbcopy", &[], text) {
        "pbcopy"
    } else if pipe_to("clip.exe", &[], text) {
        "clip.exe"
    } else {
        ""
    };
    if !system.is_empty() {
        method = system;
    }

    // OSC 52: inside tmux plain OSC 52 is handled by tmux itself (set-clipboard on)
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let seq = format!("\x1b]52;c;{b64}\x07");
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
    if method.is_empty() {
        "OSC 52"
    } else {
        method
    }
}

fn color_sgr(c: Color, fg: bool) -> Option<String> {
    let base = if fg { 38 } else { 48 };
    Some(match c {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => format!("{base};2;{r};{g};{b}"),
        Color::Indexed(i) => format!("{base};5;{i}"),
        named => {
            let idx = match named {
                Color::Black => 0,
                Color::Red => 1,
                Color::Green => 2,
                Color::Yellow => 3,
                Color::Blue => 4,
                Color::Magenta => 5,
                Color::Cyan => 6,
                Color::Gray => 7,
                Color::DarkGray => 8,
                Color::LightRed => 9,
                Color::LightGreen => 10,
                Color::LightYellow => 11,
                Color::LightBlue => 12,
                Color::LightMagenta => 13,
                Color::LightCyan => 14,
                _ => 15,
            };
            format!("{base};5;{idx}")
        }
    })
}

fn sgr(style: Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    let m = style.add_modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            parts.push(code.into());
        }
    }
    if let Some(c) = style.fg.and_then(|c| color_sgr(c, true)) {
        parts.push(c);
    }
    if let Some(c) = style.bg.and_then(|c| color_sgr(c, false)) {
        parts.push(c);
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", parts.join(";"))
    }
}

/// Render laid-out lines as ANSI text (or plain text when `color` is false).
pub fn lines_to_ansi(lines: &[Line], color: bool) -> String {
    let mut out = String::new();
    for l in lines {
        for s in &l.spans {
            if color {
                let code = sgr(s.style);
                if code.is_empty() {
                    out.push_str(&s.content);
                } else {
                    out.push_str(&code);
                    out.push_str(&s.content);
                    out.push_str("\x1b[0m");
                }
            } else {
                out.push_str(&s.content);
            }
        }
        // trailing padding is noise in a dump
        let trimmed = out.trim_end_matches(' ').len();
        out.truncate(trimmed);
        out.push('\n');
    }
    out
}
