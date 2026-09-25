//! The sandbox desktop image: its files, its tag, and the two tar streams
//! conduit hands to docker — the build context, and the runtime bundle copied
//! into a container before each start.
//!
//! Everything is embedded at compile time, so a conduit binary always builds
//! the image it was written against and never fetches build files at runtime.

use sha2::{Digest, Sha256};

use super::model::SandboxOs;

pub const DOCKERFILE: &str = include_str!("image/Dockerfile");
pub const ENTRYPOINT: &str = include_str!("image/entrypoint.sh");
pub const GUEST: &str = include_str!("image/guest.py");

/// Where the runtime bundle lands inside the container.
pub const RUNTIME_DIR: &str = "/opt/conduit";
pub const GUEST_PATH: &str = "/opt/conduit/guest.py";

pub const REPOSITORY: &str = "conduit-sandbox";

/// `conduit-sandbox:<os>-<hash>`, where the hash covers everything that goes
/// into the image. Editing the Dockerfile or the entrypoint changes the tag, so
/// a new conduit never mistakes an old image for its own. `guest.py` is not
/// part of it: that file is copied in at every start instead of baked in.
pub fn tag(os: SandboxOs) -> String {
    let mut hash = Sha256::new();
    for part in [
        os.key(),
        os.base_image(),
        os.firefox_source(),
        DOCKERFILE,
        ENTRYPOINT,
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let digest = hash.finalize();
    let short: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{REPOSITORY}:{}-{short}", os.key())
}

/// `docker build` for one OS, reading the context tar from stdin.
pub fn build_args(os: SandboxOs) -> Vec<String> {
    vec![
        "build".into(),
        "--progress=plain".into(),
        "--tag".into(),
        tag(os),
        "--label".into(),
        format!("{}={}", super::docker::LABEL_IMAGE, os.key()),
        "--build-arg".into(),
        format!("BASE={}", os.base_image()),
        "--build-arg".into(),
        format!("FIREFOX={}", os.firefox_source()),
        "-".into(),
    ]
}

/// The build context: the Dockerfile and the entrypoint, nothing else.
pub fn build_context() -> Vec<u8> {
    let mut tar = Tar::default();
    tar.file("Dockerfile", DOCKERFILE.as_bytes(), 0o644);
    tar.file("entrypoint.sh", ENTRYPOINT.as_bytes(), 0o755);
    tar.finish()
}

/// What `docker cp - <container>:/opt/conduit` receives before each start:
/// the guest helper, and the screen size the entrypoint reads.
pub fn runtime_bundle(width: u32, height: u32) -> Vec<u8> {
    let mut tar = Tar::default();
    tar.file("guest.py", GUEST.as_bytes(), 0o755);
    tar.file("geometry", format!("{width}x{height}\n").as_bytes(), 0o644);
    tar.finish()
}

/// A minimal ustar writer: regular files with short names, which is all these
/// two archives ever contain. Hand-rolled rather than a dependency because it
/// is forty lines and the format has not changed since 1988.
#[derive(Default)]
struct Tar {
    out: Vec<u8>,
}

impl Tar {
    fn file(&mut self, name: &str, contents: &[u8], mode: u32) {
        assert!(name.len() < 100, "tar entry names here are short");
        let mut header = [0u8; 512];
        let field = |header: &mut [u8; 512], at: usize, bytes: &[u8]| {
            header[at..at + bytes.len()].copy_from_slice(bytes);
        };
        field(&mut header, 0, name.as_bytes());
        field(&mut header, 100, format!("{mode:07o}\0").as_bytes());
        field(&mut header, 108, b"0000000\0"); // uid: root, as docker cp expects
        field(&mut header, 116, b"0000000\0"); // gid
        field(
            &mut header,
            124,
            format!("{:011o}\0", contents.len()).as_bytes(),
        );
        field(&mut header, 136, b"00000000000\0"); // mtime: fixed, so builds cache
        field(&mut header, 148, b"        "); // checksum placeholder
        header[156] = b'0'; // regular file
        field(&mut header, 257, b"ustar\0");
        field(&mut header, 263, b"00");
        let sum: u32 = header.iter().map(|&b| b as u32).sum();
        field(&mut header, 148, format!("{sum:06o}\0 ").as_bytes());

        self.out.extend_from_slice(&header);
        self.out.extend_from_slice(contents);
        let pad = (512 - contents.len() % 512) % 512;
        self.out.extend(std::iter::repeat_n(0u8, pad));
    }

    fn finish(mut self) -> Vec<u8> {
        self.out.extend(std::iter::repeat_n(0u8, 1024));
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the archive back with the same rules a real tar reader applies:
    /// every header's checksum must hold, sizes must match, and the stream must
    /// end with two zero blocks.
    fn entries(archive: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
        assert_eq!(archive.len() % 512, 0);
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let header = &archive[at..at + 512];
            if header.iter().all(|&b| b == 0) {
                assert!(
                    archive[at..].iter().all(|&b| b == 0),
                    "data after end marker"
                );
                assert!(archive.len() - at >= 1024, "missing end-of-archive blocks");
                break;
            }
            let text = |from: usize, len: usize| {
                String::from_utf8_lossy(&header[from..from + len])
                    .trim_matches(|c: char| c == '\0' || c == ' ')
                    .to_string()
            };
            let stored = u32::from_str_radix(&text(148, 8), 8).unwrap();
            let mut blank = header.to_vec();
            blank[148..156].copy_from_slice(b"        ");
            let computed: u32 = blank.iter().map(|&b| b as u32).sum();
            assert_eq!(stored, computed, "bad checksum");
            assert_eq!(&header[257..263], b"ustar\0");
            let name = text(0, 100);
            let mode = u32::from_str_radix(&text(100, 8), 8).unwrap();
            let size = usize::from_str_radix(&text(124, 12), 8).unwrap();
            let body = archive[at + 512..at + 512 + size].to_vec();
            out.push((name, mode, body));
            at += 512 + size.div_ceil(512) * 512;
        }
        out
    }

    #[test]
    fn the_build_context_holds_exactly_the_two_build_files() {
        let got = entries(&build_context());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "Dockerfile");
        assert_eq!(got[0].2, DOCKERFILE.as_bytes());
        assert_eq!(got[1].0, "entrypoint.sh");
        // The Dockerfile copies it without --chmod, so the mode must travel
        // in the archive or the entrypoint cannot execute.
        assert_eq!(got[1].1, 0o755);
    }

    #[test]
    fn the_runtime_bundle_carries_the_helper_and_the_screen_size() {
        let got = entries(&runtime_bundle(1440, 900));
        assert_eq!(got[0].0, "guest.py");
        assert_eq!(got[0].1, 0o755);
        assert_eq!(got[0].2, GUEST.as_bytes());
        assert_eq!(got[1].0, "geometry");
        assert_eq!(got[1].2, b"1440x900\n");
    }

    #[test]
    fn tags_are_stable_per_os_and_distinct_across_them() {
        let tags: Vec<_> = SandboxOs::ALL.iter().map(|os| tag(*os)).collect();
        assert_eq!(
            tags[0],
            tag(SandboxOs::Ubuntu2404),
            "tags must be deterministic"
        );
        for (i, a) in tags.iter().enumerate() {
            assert!(a.starts_with("conduit-sandbox:"), "{a}");
            for b in &tags[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Docker tag grammar: [A-Za-z0-9_][A-Za-z0-9_.-]{0,127}
        let t = tags[0].split_once(':').unwrap().1;
        assert!(t.len() <= 128);
        assert!(
            t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        );
    }

    #[test]
    fn build_reads_its_context_from_stdin_with_the_os_arguments() {
        let args = build_args(SandboxOs::Debian12);
        assert_eq!(args.last().unwrap(), "-");
        assert!(args.contains(&"BASE=debian:12".to_string()));
        assert!(args.contains(&"FIREFOX=esr".to_string()));
        assert!(args.contains(&tag(SandboxOs::Debian12)));
    }
}
