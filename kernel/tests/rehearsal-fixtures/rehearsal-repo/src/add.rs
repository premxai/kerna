/// Deterministic rehearsal target for `scripts/accept-contained-run.ps1`.
/// The body returns zero on purpose; the proof is that a governed Claude Code
/// edit inside a disposable worktree changes it to `a + b`.
pub fn add(a: i32, b: i32) -> i32 {
    let _ = (a, b);
    0
}

#[cfg(test)]
mod tests {
    use super::add;

    #[test]
    fn adds_the_operands() {
        assert_eq!(add(2, 3), 5);
    }
}
