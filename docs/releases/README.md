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

## After the release

The release job signs `SHA256SUMS` (`SHA256SUMS.asc`, see `packaging/ci/sign-checksums.sh`) and
publishes the packages only when the reproducibility job built the same packages again.

Then check the AppImage update on a separate test machine with the previous release:

```sh
scripts/release/appimage-update-check.sh <previous version> <new version> ~/aiu-test
```

It downloads the previous AppImage and a fixed version of AppImageUpdate (both checked against
their SHA-256), updates the AppImage to the newest release through its `.zsync` file and checks the
result against the new release's `SHA256SUMS` and its signature. 0.8.19 is the first release whose
AppImage carries the update information; older ones are updated by downloading the new AppImage.
