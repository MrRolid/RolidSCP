//! Print the licences of the syntax definitions built into the viewer
//! (from two-face) in the Debian copyright format; packaging/build-deb.sh
//! appends them to the copyright file of the package.

fn main() {
    let list = two_face::acknowledgement::listing();
    for l in list.for_syntaxes().iter().chain(list.for_themes()) {
        println!();
        println!("Files: two-face/{}", l.rel_path.display());
        println!("Comment: syntax highlighting definition built into the F3 viewer (crate two-face)");
        println!("License: {:?}", l.ty);
        for line in l.text.lines() {
            if line.trim().is_empty() {
                println!(" .");
            } else {
                println!(" {line}");
            }
        }
    }
}
