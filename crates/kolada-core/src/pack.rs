//! The files the Windows installer carries, appended to its program
//! (`tools/setup`), and read back by the app when it installs them
//! (`kolada-win/src/setup.rs`):
//!
//! ```text
//! stub | for each file: name length (u16), name, size (u64), bytes | count (u32), payload length (u64), MAGIC
//! ```
//!
//! Numbers are little-endian. The stub is everything before the payload.

pub const MAGIC: &[u8; 8] = b"NOTSYPK1";
const TRAILER: usize = 4 + 8 + 8;

/// A file the installer puts in place: its name in the install folder and
/// its bytes.
#[derive(Debug, PartialEq)]
pub struct File {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// `stub` with `files` after it.
pub fn pack(stub: &[u8], files: &[File]) -> Vec<u8> {
    let mut body = Vec::new();
    for f in files {
        body.extend_from_slice(&(f.name.len() as u16).to_le_bytes());
        body.extend_from_slice(f.name.as_bytes());
        body.extend_from_slice(&(f.bytes.len() as u64).to_le_bytes());
        body.extend_from_slice(&f.bytes);
    }
    let mut out = Vec::with_capacity(stub.len() + body.len() + TRAILER);
    out.extend_from_slice(stub);
    out.extend_from_slice(&body);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(MAGIC);
    out
}

/// The stub, without any payload.
pub fn stub(exe: &[u8]) -> &[u8] {
    match layout(exe) {
        Some((start, _)) => &exe[..start],
        None => exe,
    }
}

/// Where the payload starts, and how many files it holds.
fn layout(exe: &[u8]) -> Option<(usize, usize)> {
    if exe.len() < TRAILER || &exe[exe.len() - 8..] != MAGIC {
        return None;
    }
    let t = &exe[exe.len() - TRAILER..];
    let count = u32::from_le_bytes(t[..4].try_into().ok()?) as usize;
    let len = u64::from_le_bytes(t[4..12].try_into().ok()?) as usize;
    let start = exe.len().checked_sub(TRAILER)?.checked_sub(len)?;
    Some((start, count))
}

/// The files `exe` carries; None for a stub alone or a damaged payload.
pub fn unpack(exe: &[u8]) -> Option<Vec<File>> {
    let (start, count) = layout(exe)?;
    let mut at = &exe[start..exe.len() - TRAILER];
    let mut files = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let n = u16::from_le_bytes(at.get(..2)?.try_into().ok()?) as usize;
        let name = std::str::from_utf8(at.get(2..2 + n)?).ok()?.to_string();
        at = &at[2 + n..];
        let size = u64::from_le_bytes(at.get(..8)?.try_into().ok()?) as usize;
        let bytes = at.get(8..8 + size)?.to_vec();
        at = &at[8 + size..];
        // A plain name, never a path out of the install folder.
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return None;
        }
        files.push(File { name, bytes });
    }
    at.is_empty().then_some(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_its_files_and_gives_back_the_stub() {
        // What tools/setup.ps1 packs: the app, its writing helper (a second
        // program) and the notices. Never a model.
        let files = vec![
            File { name: "kolada.exe".into(), bytes: b"MZ the app".to_vec() },
            File { name: "kolada-ai.exe".into(), bytes: b"MZ the helper".to_vec() },
            File { name: "THIRD-PARTY.md".into(), bytes: b"# licenses".to_vec() },
            File { name: "LICENSE-llama.cpp.txt".into(), bytes: b"MIT License".to_vec() },
        ];
        let exe = pack(b"MZ stub", &files);
        assert_eq!(unpack(&exe), Some(files));
        assert_eq!(stub(&exe), b"MZ stub");
        assert_eq!(unpack(stub(&exe)), None, "the uninstaller carries nothing");
        assert_eq!(stub(b"MZ stub"), b"MZ stub");
    }

    #[test]
    fn refuses_a_damaged_payload_or_a_path() {
        let exe = pack(b"MZ", &[File { name: "kolada.exe".into(), bytes: vec![7; 100] }]);
        for cut in [1, 10, 50] {
            let mut bad = exe.clone();
            bad.drain(10..10 + cut);
            assert_eq!(unpack(&bad), None, "{cut} bytes missing");
        }
        for name in ["..\\evil.exe", "C:evil.exe", "sub/kolada.exe", ""] {
            assert_eq!(unpack(&pack(b"MZ", &[File { name: name.into(), bytes: vec![1] }])), None, "{name:?}");
        }
    }
}
