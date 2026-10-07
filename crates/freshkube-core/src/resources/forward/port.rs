//! The local end of a forward: which port, and listening on it on both
//! loopback addresses and nowhere else.
//!
//! The automatic port keeps the remote port's digits: 10000 + the remote
//! port below 10000 (3306 → 13306), else the remote port itself. When that
//! is taken, the first digit steps (23306, 33306, 43306), and past that the
//! system picks one.
//!
//! Binding first goes without `SO_REUSEADDR`. With it, macOS lets
//! 127.0.0.1:p bind while another program listens on *:p, and the more
//! specific socket then takes that program's loopback traffic. Without it,
//! though, a port stays refused for half a minute after a forward whose
//! connections it closed itself, while they sit in TIME_WAIT. So a refused
//! port is tried once more with `SO_REUSEADDR` when nothing accepts a
//! connection on it and none of our forwards holds it.
//!
//! Linux checks the flag on both sockets, and a connection takes it from
//! the listener that accepted it, so there the retry can't pass our own
//! TIME_WAIT. On Linux the first bind sets the flag: it never binds over
//! a listening socket, and our connections then wait out TIME_WAIT
//! without holding the port. Windows must never set it first, since
//! there it lets a socket bind over a port another program listens on.
//! Without it, Windows passes our own TIME_WAIT, but it also lets
//! 127.0.0.1:p bind while another program holds *:p, even when our socket
//! asks for the address exclusively (`SO_EXCLUSIVEADDRUSE`). So on Windows
//! a bind first tries the wildcard address itself, exclusively, and drops
//! it at once: that is refused while anything holds the port. The
//! listener still binds exclusively, so no one can bind over it.
//!
//! Windows refuses a port it reserves with `PermissionDenied`, not
//! `AddrInUse`; there the automatic rule steps past it as past a taken one.
//!
//! ::1 is left out only when the machine has no IPv6 loopback. Any other
//! failure there fails the bind, so localhost:p never reaches whatever
//! holds ::1:p.

use std::collections::HashSet;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use tokio::net::{TcpListener, TcpSocket};

/// How many connections may wait to be accepted.
const BACKLOG: u32 = 128;
/// Tries at a system-picked port that ::1 also has free.
const SYSTEM_TRIES: usize = 8;
/// Whether the first bind sets `SO_REUSEADDR`: only on Linux, where it
/// can't bind over a listener.
const REUSE_FIRST: bool = cfg!(target_os = "linux");
/// How long asking whether something listens may take. On the loopback
/// the answer is immediate, unless a listener is too busy to accept, or on
/// Windows, which tries a refused loopback connection again for about a
/// second before it reports the refusal.
const PROBE_DEADLINE: Duration = if cfg!(windows) {
    Duration::from_millis(2500)
} else {
    Duration::from_millis(200)
};

/// The ports our forwards listen on, so that a probe never reaches one.
static HELD: Mutex<Option<HashSet<u16>>> = Mutex::new(None);

/// The port the automatic rule tries first.
pub fn preferred_port(remote: u16) -> u16 {
    if remote < 10_000 {
        remote + 10_000
    } else {
        remote
    }
}

/// The ports the automatic rule tries in turn, before the system picks
/// one: the first digit steps from 1 to 4 below 10000.
pub fn candidates(remote: u16) -> Vec<u16> {
    if remote < 10_000 {
        (1..=4).map(|digit| digit * 10_000 + remote).collect()
    } else {
        vec![remote]
    }
}

/// Listening on one port of 127.0.0.1 and, when the Mac has it, ::1.
#[derive(Debug)]
pub struct Listeners {
    pub port: u16,
    pub v4: TcpListener,
    pub v6: Option<TcpListener>,
    _held: Held,
}

/// Marks a port as one of ours while it lives.
#[derive(Debug)]
struct Held(u16);

impl Drop for Held {
    fn drop(&mut self) {
        held(|ports| ports.remove(&self.0));
    }
}

fn held<T>(change: impl FnOnce(&mut HashSet<u16>) -> T) -> T {
    let mut ports = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    change(ports.get_or_insert_with(HashSet::new))
}

/// Listens on `port` of both loopback addresses, or fails with
/// `AddrInUse` if either is taken. Port 0 lets the system pick one that
/// both have free.
pub fn bind_loopback(port: u16) -> io::Result<Listeners> {
    if port != 0 {
        return bind_both(port);
    }
    let mut last = None;
    for _ in 0..SYSTEM_TRIES {
        match bind_both(0) {
            Ok(listeners) => return Ok(listeners),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::AddrInUse.into()))
}

/// Listens on the first free port of the automatic rule for `remote`.
pub fn bind_automatic(remote: u16) -> io::Result<Listeners> {
    for port in candidates(remote) {
        match bind_both(port) {
            Ok(listeners) => return Ok(listeners),
            Err(error) if is_taken(&error) || is_reserved(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    bind_loopback(0)
}

/// Another program, or another forward, listens on the port.
pub fn is_taken(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AddrInUse
}

/// Windows keeps the port from us: one in a range it reserves.
fn is_reserved(error: &io::Error) -> bool {
    cfg!(windows) && error.kind() == io::ErrorKind::PermissionDenied
}

/// The system has no IPv6 loopback: ::1 isn't configured, or IPv6 isn't
/// there at all.
fn no_ipv6(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::AddrNotAvailable | io::ErrorKind::Unsupported
    ) || error.raw_os_error() == Some(NO_ADDRESS_FAMILY)
}

/// "Address family not supported", which std leaves uncategorized.
#[cfg(unix)]
const NO_ADDRESS_FAMILY: i32 = libc::EAFNOSUPPORT;
#[cfg(windows)]
const NO_ADDRESS_FAMILY: i32 = windows_sys::Win32::Networking::WinSock::WSAEAFNOSUPPORT;

fn bind_both(port: u16) -> io::Result<Listeners> {
    // Keep the check, both binds and reservation together: another bind
    // must not probe a listener we have just opened.
    let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let ports = held.get_or_insert_with(HashSet::new);
    let v4 = bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)), ports)?;
    let port = v4.local_addr()?.port();
    let v6 = match bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port)), ports) {
        Ok(listener) => Some(listener),
        // No IPv6 loopback on this machine: 127.0.0.1 alone will do.
        Err(error) if no_ipv6(&error) => None,
        // Taken, refused or anything else: serving 127.0.0.1 alone would
        // leave localhost:p to whatever holds ::1:p.
        Err(error) => return Err(error),
    };
    ports.insert(port);
    Ok(Listeners {
        port,
        v4,
        v6,
        _held: Held(port),
    })
}

/// Listens on `address`, reusing it only when what holds it is no
/// listener: closed connections waiting out TIME_WAIT.
fn bind(address: SocketAddr, ports: &HashSet<u16>) -> io::Result<TcpListener> {
    match free_everywhere(address).and_then(|()| listen(address, false)) {
        Err(error)
            if is_taken(&error)
                && address.port() != 0
                && !ports.contains(&address.port())
                && !accepts(address) =>
        {
            listen(address, true)
        }
        bound => bound,
    }
}

/// Something listens on `address`. Only a refusal says nothing does.
fn accepts(address: SocketAddr) -> bool {
    !matches!(
        std::net::TcpStream::connect_timeout(&address, PROBE_DEADLINE),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused
    )
}

fn listen(address: SocketAddr, reuse: bool) -> io::Result<TcpListener> {
    let socket = match address {
        SocketAddr::V4(_) => TcpSocket::new_v4()?,
        SocketAddr::V6(_) => TcpSocket::new_v6()?,
    };
    socket.set_reuseaddr(reuse || REUSE_FIRST)?;
    #[cfg(windows)]
    if !reuse {
        exclusive(&socket)?;
    }
    socket.bind(address)?;
    socket.listen(BACKLOG)
}

/// Nothing holds `address`'s port on any address of its family. Windows
/// alone needs asking: elsewhere the bind itself refuses a port another
/// program holds on the wildcard address.
#[cfg(windows)]
fn free_everywhere(address: SocketAddr) -> io::Result<()> {
    if address.port() == 0 {
        return Ok(());
    }
    let (socket, wildcard) = match address {
        SocketAddr::V4(_) => (TcpSocket::new_v4()?, Ipv4Addr::UNSPECIFIED.into()),
        SocketAddr::V6(_) => (TcpSocket::new_v6()?, Ipv6Addr::UNSPECIFIED.into()),
    };
    exclusive(&socket)?;
    // Bound, never listening, and closed again on return.
    socket.bind(SocketAddr::new(wildcard, address.port()))
}

#[cfg(not(windows))]
fn free_everywhere(_address: SocketAddr) -> io::Result<()> {
    Ok(())
}

/// Asks Windows for the address alone, so the bind fails while another
/// socket holds the port, on the same address or the wildcard.
#[cfg(windows)]
fn exclusive(socket: &TcpSocket) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        SO_EXCLUSIVEADDRUSE, SOCKET_ERROR, SOL_SOCKET, setsockopt,
    };
    let on: i32 = 1;
    // SAFETY: the socket is open for the call, and `on` outlives it.
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            (&on as *const i32).cast(),
            std::mem::size_of::<i32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_port_keeps_the_remote_digits() {
        assert_eq!(preferred_port(3306), 13306);
        assert_eq!(preferred_port(5432), 15432);
        assert_eq!(preferred_port(80), 10080);
        assert_eq!(preferred_port(8080), 18080);
        assert_eq!(preferred_port(9999), 19999);
        assert_eq!(
            preferred_port(10_250),
            10_250,
            "five digits stay as they are"
        );
        assert_eq!(preferred_port(65_535), 65_535);
        assert_eq!(candidates(3306), vec![13306, 23306, 33306, 43306]);
        assert_eq!(candidates(1), vec![10001, 20001, 30001, 40001]);
        assert_eq!(candidates(27_017), vec![27_017]);
    }

    /// A port nothing listens on, found by asking the system.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn a_port_is_bound_on_both_loopbacks_and_given_back_on_drop() {
        let port = free_port();
        let listeners = bind_loopback(port).unwrap();
        assert_eq!(listeners.port, port);
        assert_eq!(
            listeners.v4.local_addr().unwrap(),
            SocketAddr::from((Ipv4Addr::LOCALHOST, port))
        );
        if let Some(v6) = &listeners.v6 {
            assert_eq!(
                v6.local_addr().unwrap(),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port))
            );
        }
        let again = bind_loopback(port).unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AddrInUse, "ours until dropped");
        drop(listeners);
        bind_loopback(port).expect("free again once dropped");
    }

    #[tokio::test]
    async fn another_programs_wildcard_listener_makes_the_port_taken() {
        // std sets SO_REUSEADDR, as many servers do. Ours is refused, and
        // as the other program accepts connections the port isn't reused,
        // which would take that program's loopback traffic.
        let wildcard = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = wildcard.local_addr().unwrap().port();
        let error = bind_loopback(port).unwrap_err();
        assert!(is_taken(&error), "{error}");
    }

    #[tokio::test]
    async fn a_port_whose_closed_connections_wait_out_time_wait_is_bound_again() {
        let listeners = bind_loopback(0).unwrap();
        let port = listeners.port;
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (accepted, _) = listeners.v4.accept().await.unwrap();
        // Closing first, as a stopped forward does, leaves our side in
        // TIME_WAIT.
        drop(accepted);
        let _ = std::io::Read::read(&mut client, &mut [0; 1]);
        drop(client);
        drop(listeners);
        // On macOS the first bind, without the flag, is refused, which is
        // why the retry exists; on Linux it sets the flag, as the closed
        // connections did, and on Windows an exclusive bind passes our
        // TIME_WAIT, so both pass at once.
        assert_eq!(
            listen(SocketAddr::from((Ipv4Addr::LOCALHOST, port)), false).is_ok(),
            cfg!(any(target_os = "linux", windows))
        );
        bind_loopback(port).expect("nothing listens, so the port is reused");
    }

    #[tokio::test]
    async fn a_live_port_is_refused_even_with_reuse() {
        // Our own listener, with connections closed from our side first, so
        // that some sit in TIME_WAIT, and one still open.
        let listeners = bind_loopback(0).unwrap();
        let port = listeners.port;
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        for _ in 0..3 {
            let mut client = std::net::TcpStream::connect(address).unwrap();
            let (accepted, _) = listeners.v4.accept().await.unwrap();
            drop(accepted);
            let _ = std::io::Read::read(&mut client, &mut [0; 1]);
        }
        let _open = std::net::TcpStream::connect(address).unwrap();
        let _accepted = listeners.v4.accept().await.unwrap();
        let error = bind_loopback(port).unwrap_err();
        assert!(is_taken(&error), "{error}");
        assert!(listen(address, true).is_err(), "not even with the flag");

        // Another program's listener on the loopback, with the flag set as
        // std sets it.
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = other.local_addr().unwrap().port();
        let error = bind_loopback(port).unwrap_err();
        assert!(is_taken(&error), "{error}");
        assert!(listen(SocketAddr::from((Ipv4Addr::LOCALHOST, port)), true).is_err());
    }

    #[tokio::test]
    async fn a_taken_preferred_port_steps_the_first_digit() {
        // Find a remote port whose first two candidates are free, then take
        // the first.
        let remote = (2000..9000)
            .find(|remote| {
                candidates(*remote)
                    .iter()
                    .take(2)
                    .all(|port| std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok())
            })
            .unwrap();
        let first = bind_loopback(preferred_port(remote)).unwrap();
        let second = bind_automatic(remote).unwrap();
        assert_eq!(first.port, remote + 10_000);
        assert_eq!(second.port, remote + 20_000);
    }

    #[tokio::test]
    async fn concurrent_binds_do_not_probe_their_listeners() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(32));
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let barrier = barrier.clone();
            tasks.push(tokio::task::spawn_blocking(move || {
                barrier.wait();
                let listeners = bind_automatic(9876);
                // Every task reaches the second wait, bound or not, so a
                // failed bind fails the test instead of leaving the others
                // waiting.
                barrier.wait();
                let listeners = listeners.unwrap();
                let v4 = listeners.v4.into_std().unwrap();
                assert!(matches!(v4.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock), "a bind must not open a connection to another forward");
                if let Some(v6) = listeners.v6 {
                    let v6 = v6.into_std().unwrap();
                    assert!(matches!(v6.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
    }

    #[test]
    fn only_a_missing_ipv6_loopback_leaves_out_ipv6() {
        assert!(no_ipv6(&io::ErrorKind::AddrNotAvailable.into()));
        assert!(no_ipv6(&io::Error::from_raw_os_error(NO_ADDRESS_FAMILY)));
        assert!(!no_ipv6(&io::ErrorKind::PermissionDenied.into()));
        assert!(!no_ipv6(&io::ErrorKind::AddrInUse.into()));
    }

    #[tokio::test]
    async fn port_zero_lets_the_system_pick() {
        let listeners = bind_loopback(0).unwrap();
        assert_ne!(listeners.port, 0);
        assert_eq!(listeners.v4.local_addr().unwrap().port(), listeners.port);
    }
}
