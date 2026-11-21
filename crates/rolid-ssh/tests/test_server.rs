//! The throwaway OpenSSH server of the integration tests itself: it must be
//! the server the tests talk to. Its port used to be taken from the range the
//! system gives to other sockets, and readiness was checked by connecting to
//! it: when another socket got the port first, sshd failed to start, the check
//! connected to that other socket, and a test talked to the wrong server
//! ("Disconnected", "Connection reset by peer", a restart never noticed).

mod sshd;

use rolid_ssh::SshConnection;

/// A port nobody listens on, below the system's port range for other sockets.
fn taken_port() -> (std::net::TcpListener, u16) {
    let low: u16 = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|r| r.split_whitespace().next()?.parse().ok())
        .unwrap_or(32768);
    for port in (low - 5000..low).rev() {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return (l, port);
        }
    }
    panic!("no free port");
}

#[test]
fn a_taken_port_is_not_mistaken_for_the_server() {
    let (_other, port) = taken_port();
    let Some(r) = sshd::Sshd::start_on("taken", port) else { return };
    let log = r.err().expect("sshd cannot listen on a port another socket has");
    assert!(log.contains("Cannot bind any address"), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_moves_off_a_port_taken_meanwhile() {
    let Some(mut srv) = sshd::Sshd::start("moves") else { return };
    let old = srv.port;
    srv.kill();
    let _other = std::net::TcpListener::bind(("127.0.0.1", old)).expect("the old port is free after the kill");
    let new = srv.restart();
    assert_ne!(new, old);
    let conn = SshConnection::connect(&srv.params(new)).await.expect("the restarted server is ours");
    drop(conn);
}

#[test]
fn the_port_is_outside_the_system_range() {
    let Some(srv) = sshd::Sshd::start("range") else { return };
    let low: u16 = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|r| r.split_whitespace().next()?.parse().ok())
        .unwrap_or(32768);
    assert!(srv.port < low, "{} ≥ {low}", srv.port);
}
