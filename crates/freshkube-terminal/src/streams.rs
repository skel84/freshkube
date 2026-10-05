//! Synthetic byte streams for tests and the stress harness, from the exec
//! spike (docs/POD_EXEC.md): plain and coloured log lines, a `top`-like
//! full-screen redraw, and wide and combining characters.

use std::fmt::Write;

/// Plain lines, like a log tail or `cat` of a big file.
#[cfg(test)]
pub(crate) fn plain(from: usize, lines: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in from..from + lines {
        let _ = write!(
            out,
            "{i:>8} level=info msg=\"request served\" path=/api/v1/pods duration=1.4ms peer=10.0.0.{}\r\n",
            i % 255
        );
    }
    out.into_bytes()
}

/// Lines with 256-colour and truecolour SGR every few words, like `ls
/// --color` or a coloured logger.
pub fn coloured(from: usize, lines: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in from..from + lines {
        let _ = write!(
            out,
            "\x1b[38;5;{}m{i:>8}\x1b[0m \x1b[1;32mINFO\x1b[0m \x1b[38;2;{};{};200mcomponent\x1b[0m \x1b[4mmessage\x1b[24m number {i} with \x1b[7mreverse\x1b[27m text\r\n",
            i % 256,
            i % 200,
            (i * 7) % 200
        );
    }
    out.into_bytes()
}

/// A `top`-like frame on the alternate screen: home the cursor and rewrite
/// every row with absolute moves and colours.
pub fn top_frame(frame: usize, columns: usize, rows: usize) -> Vec<u8> {
    let mut out = String::new();
    if frame == 0 {
        out.push_str("\x1b[?1049h\x1b[?25l");
    }
    out.push_str("\x1b[H");
    let _ = write!(
        out,
        "\x1b[1;37;44m top - frame {frame:<8} load average: 0.{:02}, 0.52, 0.41",
        frame % 100
    );
    out.push_str("\x1b[K\x1b[0m");
    for row in 2..=rows {
        let _ = write!(out, "\x1b[{row};1H");
        let pid = 1000 + row;
        let cpu = (row * 13 + frame * 7) % 100;
        let colour = if cpu > 80 {
            31
        } else if cpu > 50 {
            33
        } else {
            32
        };
        let mut line = format!(
            "{pid:>6} root      20   0  1.2g  0.3g  S \x1b[{colour}m{cpu:>3}.0\x1b[0m  2.1  12:{:02}.{:02} process-{row}",
            frame % 60,
            row % 60
        );
        line.truncate(columns.min(line.len()));
        out.push_str(&line);
        out.push_str("\x1b[K");
    }
    out.into_bytes()
}

/// Wide and combining characters, emoji and box drawing, with lines long
/// enough to wrap.
pub(crate) fn unicode(lines: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..lines {
        let _ = write!(
            out,
            "{i:>5} │ 日本語のテキスト ─ émoji 🚀✅ ─ combining e\u{301} ─ ┌─┐└─┘ {}\r\n",
            "wrap ".repeat(i % 40)
        );
    }
    out.into_bytes()
}

/// One screen to look at: the sixteen colours as text and as backgrounds,
/// each style, a 256-colour ramp and wide characters, then a prompt.
#[cfg(feature = "stress")]
pub fn sample() -> Vec<u8> {
    let mut out = String::new();
    for (row, base) in [(0, 30), (1, 90)] {
        for colour in 0..8 {
            let _ = write!(
                out,
                "\x1b[{}m colour{} \x1b[0m",
                base + colour,
                row * 8 + colour
            );
        }
        out.push_str("\r\n");
        for colour in 0..8 {
            let _ = write!(
                out,
                "\x1b[{}m colour{} \x1b[0m",
                base + 10 + colour,
                row * 8 + colour
            );
        }
        out.push_str("\r\n");
    }
    out.push_str(
        "\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \
         \x1b[4:3mundercurl\x1b[0m \x1b[9mstrikethrough\x1b[0m \x1b[7minverse\x1b[0m \
         \x1b[1;31mbold red\x1b[0m\r\n",
    );
    for index in 16..232 {
        let _ = write!(out, "\x1b[48;5;{index}m ");
        if (index - 16) % 72 == 71 {
            out.push_str("\x1b[0m\r\n");
        }
    }
    out.push_str("\x1b[0m\r\n");
    out.push_str(&String::from_utf8_lossy(&unicode(6)));
    out.push_str("\x1b[1;32mroot@web-0\x1b[0m:\x1b[1;34m/app\x1b[0m# ");
    out.into_bytes()
}
