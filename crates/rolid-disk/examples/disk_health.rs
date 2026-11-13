//! Print the disks and SMART state of a path: `cargo run -p rolid-disk --example disk_health -- /home`.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/".into());
    let m = rolid_disk::mounts::mount_of(&path);
    println!("mount: {m:?}");
    match rolid_disk::health::disks_of_path(&path) {
        Err(e) => println!("no SMART: {e:?}"),
        Ok(disks) => {
            let dir = std::env::temp_dir().join("rolid-disk-example.json");
            for d in disks {
                if let Some(u) = rolid_disk::udisks::drive(&d) {
                    println!("{d}: UDisks2 drive {:?} serial {:?} bus {:?}, SMART: {}", u.model, u.serial, u.bus, u.smart().is_some());
                }
                let h = rolid_disk::health::read_disk(&d, &dir, true);
                match h.result {
                    Ok(r) => println!("{d}: {:?} {:?}\n{:#?}", r.eval.level, r.eval.reason, r.report),
                    Err(e) => println!("{d}: no SMART: {e:?}"),
                }
            }
        }
    }
}
