//! The example shell, which Start runs in example mode instead of an exec.
//! It edits a line as a shell's line editor would, answers a handful of
//! commands, and draws a full screen and a colour test, so UI tests and
//! screenshots exercise the terminal without a cluster.

use crate::terminal::TerminalSize;

/// What the example shell sends back for some input.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Reply {
    pub(super) bytes: Vec<u8>,
    /// The shell exited, with this code.
    pub(super) exit: Option<i32>,
}

/// Where the shell is in an escape sequence the terminal sent, such as an
/// arrow key, which it skips.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Escape {
    None,
    Started,
    Csi,
    Ss3,
}

pub(super) struct ExampleShell {
    host: String,
    container: String,
    size: TerminalSize,
    line: Vec<u8>,
    escape: Escape,
    /// `top` is showing on the alternate screen.
    top: bool,
}

const HELP: &str = "\
echo TEXT    prints TEXT\r
ls, pwd, whoami, hostname\r
stty size    the terminal's rows and columns\r
colors       a colour test\r
top          a full-screen view; q leaves it\r
clear        clears the screen\r
exit [CODE]  ends the shell\r
";

impl ExampleShell {
    pub(super) fn new(host: &str, container: &str, size: TerminalSize) -> Self {
        Self {
            host: host.to_owned(),
            container: container.to_owned(),
            size,
            line: Vec::new(),
            escape: Escape::None,
            top: false,
        }
    }

    /// The greeting, the title and the first prompt.
    pub(super) fn start(&self) -> Vec<u8> {
        let mut bytes = format!(
            "\x1b]0;root@{}: /\x07Example shell in {} of {}. Nothing here reaches a cluster.\r\nType help for its commands.\r\n\r\n",
            self.host, self.container, self.host
        )
        .into_bytes();
        bytes.extend(self.prompt());
        bytes
    }

    pub(super) fn resize(&mut self, size: TerminalSize) -> Vec<u8> {
        self.size = size;
        if self.top {
            self.draw_top()
        } else {
            Vec::new()
        }
    }

    pub(super) fn input(&mut self, bytes: &[u8]) -> Reply {
        let mut reply = Reply::default();
        for &byte in bytes {
            if reply.exit.is_some() {
                break;
            }
            self.byte(byte, &mut reply);
        }
        reply
    }

    fn prompt(&self) -> Vec<u8> {
        format!(
            "\x1b[1;32mroot@{}\x1b[0m:\x1b[1;34m/\x1b[0m# ",
            self.container
        )
        .into_bytes()
    }

    fn byte(&mut self, byte: u8, reply: &mut Reply) {
        match (self.escape, byte) {
            (Escape::None, 0x1b) => return self.escape = Escape::Started,
            (Escape::Started, b'[') => return self.escape = Escape::Csi,
            (Escape::Started, b'O') => return self.escape = Escape::Ss3,
            // Option as Meta: skip the key it modified.
            (Escape::Started, _) | (Escape::Ss3, _) => return self.escape = Escape::None,
            (Escape::Csi, 0x40..=0x7e) => return self.escape = Escape::None,
            (Escape::Csi, _) => return,
            (Escape::None, _) => {}
        }
        if self.top {
            if matches!(byte, b'q' | 0x03) {
                self.top = false;
                reply.bytes.extend(b"\x1b[?1049l");
                reply.bytes.extend(self.prompt());
            }
            return;
        }
        match byte {
            b'\r' | b'\n' => {
                reply.bytes.extend(b"\r\n");
                let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
                self.run(line.trim(), reply);
                if reply.exit.is_none() && !self.top {
                    reply.bytes.extend(self.prompt());
                }
            }
            // Control-C drops the line.
            0x03 => {
                self.line.clear();
                reply.bytes.extend(b"^C\r\n");
                reply.bytes.extend(self.prompt());
            }
            // Control-D on an empty line ends the shell.
            0x04 if self.line.is_empty() => {
                reply.bytes.extend(b"exit\r\n");
                reply.exit = Some(0);
            }
            // Control-L clears the screen and keeps the line.
            0x0c => {
                reply.bytes.extend(b"\x1b[H\x1b[2J");
                reply.bytes.extend(self.prompt());
                reply.bytes.extend(&self.line);
            }
            0x7f | 0x08 => {
                // A whole character: its continuation bytes, then its lead.
                while self.line.last().is_some_and(|byte| byte & 0xc0 == 0x80) {
                    self.line.pop();
                }
                if self.line.pop().is_some() {
                    reply.bytes.extend(b"\x08 \x08");
                }
            }
            0x20..=0x7e | 0x80.. => {
                self.line.push(byte);
                reply.bytes.push(byte);
            }
            _ => {}
        }
    }

    fn run(&mut self, line: &str, reply: &mut Reply) {
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let out = &mut reply.bytes;
        let mut say = |text: &str| {
            out.extend(text.as_bytes());
            out.extend(b"\r\n");
        };
        match command {
            "" => {}
            "help" => out.extend(HELP.as_bytes()),
            "echo" => say(rest),
            "ls" => say(
                "\x1b[1;34mbin\x1b[0m  \x1b[1;32mentrypoint.sh\x1b[0m  \x1b[1;34metc\x1b[0m  \x1b[1;34mhome\x1b[0m  \x1b[1;34mtmp\x1b[0m  \x1b[1;34musr\x1b[0m  \x1b[1;34mvar\x1b[0m",
            ),
            "pwd" => say("/"),
            "whoami" => say("root"),
            "hostname" => say(&self.host),
            "stty" if rest == "size" => say(&format!("{} {}", self.size.rows, self.size.columns)),
            "colors" => out.extend(colour_test()),
            "top" => {
                self.top = true;
                out.extend(b"\x1b[?1049h");
                out.extend(self.draw_top());
            }
            "clear" => out.extend(b"\x1b[H\x1b[2J"),
            "exit" => match rest {
                "" => reply.exit = Some(0),
                code => match code.parse() {
                    Ok(code) => reply.exit = Some(code),
                    Err(_) => say(&format!("sh: exit: Illegal number: {code}")),
                },
            },
            command => say(&format!("sh: {command}: not found")),
        }
    }

    /// A full screen in the manner of `top`, fitted to the terminal.
    fn draw_top(&self) -> Vec<u8> {
        let columns = usize::from(self.size.columns);
        let fit = |text: String| -> String {
            let mut text: String = text.chars().take(columns).collect();
            let width = text.chars().count();
            text.extend(std::iter::repeat_n(' ', columns - width));
            text
        };
        let mut screen = String::from("\x1b[H\x1b[2J");
        screen += &format!(
            "\x1b[7m{}\x1b[0m\r\n",
            fit(format!(
                " top · {} · {} × {}",
                self.host, self.size.columns, self.size.rows
            ))
        );
        screen += "Mem: 412M used, 1.6G free    Load: 0.08 0.11 0.09\r\n\r\n";
        screen += &format!(
            "\x1b[1m{}\x1b[0m\r\n",
            fit("  PID USER      %CPU %MEM COMMAND".into())
        );
        for (pid, cpu, mem, command) in [
            (1, "0.3", "1.2", "/entrypoint.sh"),
            (7, "2.1", "8.4", "server --port 8080"),
            (31, "0.0", "0.4", "sh"),
            (44, "0.1", "0.2", "top"),
        ] {
            screen += &fit(format!("{pid:>5} root      {cpu:>4} {mem:>4} {command}"));
            screen += "\r\n";
        }
        screen += &format!("\x1b[{};1H\x1b[2mq leaves\x1b[0m", self.size.rows);
        screen.into_bytes()
    }
}

/// The eight colours and their bright twins as text and as backgrounds,
/// then the text attributes.
fn colour_test() -> Vec<u8> {
    let mut text = String::new();
    for base in [30, 90, 40, 100] {
        for colour in 0..8 {
            text += &format!("\x1b[{}m {colour} \x1b[0m", base + colour);
        }
        text += "\r\n";
    }
    text += "\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7minverse\x1b[0m\r\n";
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{ExampleShell, Reply};
    use crate::terminal::TerminalSize;

    fn shell() -> ExampleShell {
        ExampleShell::new(
            "web-0",
            "app",
            TerminalSize {
                columns: 100,
                rows: 30,
            },
        )
    }

    fn text(reply: &Reply) -> String {
        String::from_utf8_lossy(&reply.bytes).into_owned()
    }

    #[test]
    fn it_echoes_edits_and_answers_a_line() {
        let mut shell = shell();
        let reply = shell.input("ecno\x7f\x7fho héllo\r".as_bytes());
        let echoed = text(&reply);
        assert!(
            echoed.starts_with("ecno\x08 \x08\x08 \x08ho héllo\r\nhéllo\r\n"),
            "{echoed:?}"
        );
        assert!(echoed.ends_with("# "));
        assert_eq!(reply.exit, None);
        // Backspace takes a whole character, and arrow keys are skipped.
        let reply = shell.input("é\x7f\x1b[A\x1bOBx\r".as_bytes());
        assert!(text(&reply).starts_with("é\x08 \x08x\r\nsh: x: not found"));
    }

    #[test]
    fn stty_size_follows_a_resize() {
        let mut shell = shell();
        assert!(text(&shell.input(b"stty size\r")).contains("\r\n30 100\r\n"));
        shell.resize(TerminalSize {
            columns: 60,
            rows: 20,
        });
        assert!(text(&shell.input(b"stty size\r")).contains("\r\n20 60\r\n"));
    }

    #[test]
    fn top_takes_the_alternate_screen_until_q() {
        let mut shell = shell();
        let reply = text(&shell.input(b"top\r"));
        assert!(reply.contains("\x1b[?1049h") && reply.contains("q leaves"));
        assert!(!reply.ends_with("# "));
        // Other keys do nothing; a resize draws it again.
        assert_eq!(text(&shell.input(b"x\r")), "");
        let redrawn = shell.resize(TerminalSize {
            columns: 40,
            rows: 12,
        });
        assert!(String::from_utf8_lossy(&redrawn).contains("40 × 12"));
        let reply = text(&shell.input(b"q"));
        assert!(reply.starts_with("\x1b[?1049l") && reply.ends_with("# "));
    }

    #[test]
    fn exit_and_control_d_end_it_with_a_code() {
        assert_eq!(shell().input(b"exit 3\recho no\r").exit, Some(3));
        assert_eq!(shell().input(b"exit\r").exit, Some(0));
        assert_eq!(shell().input(b"\x04").exit, Some(0));
        // Control-D with a line typed does nothing.
        assert_eq!(shell().input(b"ls\x04").exit, None);
        let mut shell = shell();
        let reply = shell.input(b"exit x\r");
        assert_eq!(reply.exit, None);
        assert!(text(&reply).contains("Illegal number: x"));
    }
}
