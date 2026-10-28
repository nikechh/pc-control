//! Per-device results of one pcctl run, and the exit code they map to.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok(String),
    Skipped(String),
    Unverified(String),
    Failed(String),
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Skipped(_) => "skipped",
            Self::Unverified(_) => "unverified",
            Self::Failed(_) => "failed",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::Ok(d) | Self::Skipped(d) | Self::Unverified(d) | Self::Failed(d) => d,
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    entries: Vec<(String, Outcome)>,
}

impl Report {
    pub fn push(&mut self, device: impl Into<String>, outcome: Outcome) {
        self.entries.push((device.into(), outcome));
    }

    pub fn entries(&self) -> &[(String, Outcome)] {
        &self.entries
    }

    /// 0 = every device ok, 2 = some ok, 1 = none ok (an empty report included).
    pub fn exit_code(&self) -> u8 {
        let ok = self.entries.iter().filter(|(_, o)| o.is_ok()).count();
        match ok {
            0 => 1,
            n if n == self.entries.len() => 0,
            _ => 2,
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (device, outcome) in &self.entries {
            writeln!(f, "{device:<10} {:<11} {}", outcome.label(), outcome.detail())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(outcomes: &[Outcome]) -> Report {
        let mut r = Report::default();
        for (i, o) in outcomes.iter().enumerate() {
            r.push(format!("dev{i}"), o.clone());
        }
        r
    }

    #[test]
    fn exit_code_is_0_when_everything_succeeded() {
        assert_eq!(report(&[Outcome::Ok("a".into()), Outcome::Ok("b".into())]).exit_code(), 0);
    }

    #[test]
    fn exit_code_is_2_when_only_some_devices_succeeded() {
        assert_eq!(report(&[Outcome::Ok("a".into()), Outcome::Skipped("b".into())]).exit_code(), 2);
        assert_eq!(report(&[Outcome::Ok("a".into()), Outcome::Unverified("b".into())]).exit_code(), 2);
    }

    #[test]
    fn exit_code_is_1_when_nothing_succeeded() {
        assert_eq!(report(&[Outcome::Failed("a".into()), Outcome::Unverified("b".into())]).exit_code(), 1);
        assert_eq!(report(&[]).exit_code(), 1);
    }

    #[test]
    fn display_lists_one_line_per_device() {
        let text = report(&[Outcome::Ok("Direct #0000FF".into()), Outcome::Failed("boom".into())]).to_string();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().contains("ok"));
        assert!(text.contains("failed      boom"));
    }
}
