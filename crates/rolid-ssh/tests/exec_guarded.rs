//! Commands on the server run through `guarded` (search with find, folder
//! sizes with du, checksums, the viewer) must run to the end: the guard stops
//! the command when the channel's input ends, so the client must not end it
//! right away. Against a real OpenSSH server.

mod sshd;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use rolid_search::remote::guarded;
use rolid_ssh::SshConnection;

#[tokio::test(flavor = "multi_thread")]
async fn guarded_commands_run_to_the_end() {
    let Some(server) = sshd::Sshd::start("guarded") else { return };
    let conn = SshConnection::connect(&server.params(server.port)).await.unwrap();
    let no = AtomicBool::new(false);
    let mut out = Vec::new();
    let (status, _) = conn
        .exec_stream(&guarded("sleep 0.3; echo done"), &no, &mut |d| {
            out.extend_from_slice(d);
            true
        })
        .await
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out), "done\n");
    assert_eq!(status, Some(0));
    // The short check too.
    let mut out = Vec::new();
    conn.exec_probe(&guarded("sleep 0.3; echo probe"), &no, &mut |d| {
        out.extend_from_slice(d);
        true
    })
    .await
    .unwrap();
    assert_eq!(String::from_utf8_lossy(&out), "probe\n");

    // A search by name uses find on the server (not the slow walk over SFTP).
    let root = server.data_dir("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/needle.txt"), "x").unwrap();
    let fs = conn.sftp().await.unwrap();
    let q = rolid_search::Query { root: root.display().to_string(), name: "needle*".into(), ..Default::default() };
    let hits = Mutex::new(Vec::new());
    let emit = |h: rolid_search::Hit| hits.lock().unwrap().push(h.rel);
    let sink = rolid_search::Sink::new(&emit, 100, &no);
    let o = rolid_search::remote::search_remote(&conn, &fs, &q, &sink).await.unwrap();
    assert_eq!(o.method, rolid_search::Method::RemoteShell);
    assert_eq!(*hits.lock().unwrap(), ["sub/needle.txt"]);
}
