//! Process-based agent detection.
//!
//! Given a PTY fd, find the foreground process group leader and read its
//! short name from `/proc/<pgid>/comm`. Match that name against the
//! configured manifests to decide which agent (if any) is running.

use std::os::fd::RawFd;

use super::manifest::CompiledManifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub manifest_name: String,
    pub display_name: String,
    pub pid: libc::pid_t,
}

/// Return the foreground process group of the given PTY fd, or `None`
/// when the call fails (closed fd, no controlling tty, etc.).
pub fn pty_foreground_pgid(fd: RawFd) -> Option<libc::pid_t> {
    // SAFETY: tcgetpgrp is a thread-safe libc call that operates only on
    // the supplied fd; it returns -1 on error and sets errno.
    let pgid = unsafe { libc::tcgetpgrp(fd) };
    if pgid > 0 { Some(pgid) } else { None }
}

/// Read `/proc/<pid>/comm` (the kernel-supplied short process name) for
/// the given pid. Returns `None` if the file is missing or unreadable.
pub fn process_name(pid: libc::pid_t) -> Option<String> {
    use gtk4::gio;
    use gtk4::prelude::*;

    let path = format!("/proc/{pid}/comm");
    let file = gio::File::for_path(&path);
    let (bytes, _etag) = file.load_contents(gio::Cancellable::NONE).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Match a process short-name against compiled manifests. Returns the
/// first manifest whose `process_patterns` matches, with the supplied pid
/// recorded in the result.
pub fn match_manifest(
    process: &str,
    pid: libc::pid_t,
    manifests: &[CompiledManifest],
) -> Option<Detection> {
    manifests
        .iter()
        .find(|m| m.process_patterns.iter().any(|r| r.is_match(process)))
        .map(|m| Detection {
            manifest_name: m.name.clone(),
            display_name: m.display_name.clone(),
            pid,
        })
}

/// Convenience: run the full pgid → comm → manifest pipeline against a
/// single fd. Returns `None` if any step yields no detection.
pub fn detect_from_fd(fd: RawFd, manifests: &[CompiledManifest]) -> Option<Detection> {
    let pid = pty_foreground_pgid(fd)?;
    let name = process_name(pid)?;
    match_manifest(&name, pid, manifests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::manifest::builtin_manifests;

    #[test]
    fn manifest_match_picks_first_winner() {
        let manifests: Vec<CompiledManifest> = builtin_manifests()
            .into_iter()
            .map(|m| m.compile().unwrap())
            .collect();
        let hit = match_manifest("claude", 42, &manifests).unwrap();
        assert_eq!(hit.manifest_name, "claude-code");
        assert_eq!(hit.pid, 42);
    }

    #[test]
    fn manifest_match_returns_none_for_unrelated_process() {
        let manifests: Vec<CompiledManifest> = builtin_manifests()
            .into_iter()
            .map(|m| m.compile().unwrap())
            .collect();
        assert!(match_manifest("bash", 1, &manifests).is_none());
        assert!(match_manifest("vim", 1, &manifests).is_none());
    }

    #[test]
    fn process_name_of_init_pid_one() {
        // pid 1 always exists on Linux; comm is "init" or "systemd"
        // depending on the system, but it must be non-empty.
        let name = process_name(1);
        assert!(name.is_some(), "expected /proc/1/comm to be readable");
        assert!(!name.unwrap().is_empty());
    }
}
