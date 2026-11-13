//! `keyring_get <profile-id>` – prints whether a password is stored (never the password).
fn main() {
    let id = std::env::args().nth(1).expect("profile id");
    match rolid_config::secrets::get(&id) {
        Ok(Some(p)) => println!("stored ({} chars)", p.chars().count()),
        Ok(None) => println!("not stored"),
        Err(e) => println!("error: {e}"),
    }
}
