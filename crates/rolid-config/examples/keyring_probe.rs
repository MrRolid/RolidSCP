//! Check keyring availability: `cargo run -p rolid-config --example keyring_probe`
fn main() {
    let t = std::time::Instant::now();
    let id = "probe-test";
    println!(
        "set: {:?} ({:?})",
        rolid_config::secrets::set(id, "secret"),
        t.elapsed()
    );
    println!(
        "get: {:?}",
        rolid_config::secrets::get(id).map(|o| o.map(|_| "***"))
    );
    println!(
        "delete: {:?} ({:?})",
        rolid_config::secrets::delete(id),
        t.elapsed()
    );
}
