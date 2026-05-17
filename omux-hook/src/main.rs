//! omux-hook — M0 stub.
//!
//! Becomes the D-Bus-talking helper at M4. For M0 it parses argv and
//! exits 0 so the workspace builds end-to-end.

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    eprintln!("omux-hook M0 stub: argv = {args:?}");
    Ok(())
}
