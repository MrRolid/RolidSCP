# Release notes

One Markdown file per version, named after the version in `Cargo.toml` without the `v`:
`docs/releases/0.8.14.md` is the text of the GitHub release for the tag `v0.8.14`.

The workflow `.github/workflows/build.yml` uses the file as the release text when the tag is
pushed. A tag without its file fails the build before anything is published, so the file has to be
committed before the tag.

The text is shown on the release page as it is, so it is written for users, in English. A useful
shape (see [0.8.13.md](0.8.13.md)):

- one or two sentences on what RolidSCP is;
- "New in <version>": the changes users notice, fixed bugs included;
- requirements and installation of the attached packages;
- the licence.

The release is marked as a pre-release when the version (e.g. `0.9.0-beta.1`) or the label shown in
the program (`PRERELEASE` in `crates/rolid-ui/src/version.rs`) contains "beta".
