//! Opt-in abuse controls. With no policy configured every report is accepted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// When set, only these project keys are accepted.
    pub projects: Option<HashMap<String, Project>>,
    /// Reports accepted per minute for one project key from one address.
    pub rate_per_minute: Option<u32>,
    /// Total attachment bytes kept; past it new reports are refused.
    pub storage_budget_bytes: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// When not empty, the request `Origin` must be one of these.
    #[serde(default)]
    pub origins: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    UnknownProject,
    Origin,
}

impl Policy {
    pub fn parse(json: Option<&str>) -> serde_json::Result<Self> {
        json.filter(|s| !s.trim().is_empty())
            .map_or_else(|| Ok(Self::default()), serde_json::from_str)
    }

    pub fn admit(&self, project: &str, origin: Option<&str>) -> Result<(), Denied> {
        let Some(projects) = &self.projects else { return Ok(()) };
        let rules = projects.get(project).ok_or(Denied::UnknownProject)?;
        if rules.origins.is_empty() || origin.is_some_and(|o| rules.origins.iter().any(|a| a == o))
        {
            Ok(())
        } else {
            Err(Denied::Origin)
        }
    }
}

const WINDOW: Duration = Duration::from_secs(60);
const TRACKED: usize = 10_000;

#[derive(Default)]
pub struct Limiter {
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl Limiter {
    pub fn allow(&self, per_minute: Option<u32>, project: &str, address: &str) -> bool {
        let Some(limit) = per_minute else { return true };
        let now = Instant::now();
        let mut windows = self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if windows.len() >= TRACKED {
            windows.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
        }
        let (start, count) = windows.entry(format!("{project} {address}")).or_insert((now, 0));
        if now.duration_since(*start) >= WINDOW {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        *count <= limit
    }
}

#[cfg(test)]
mod tests {
    use super::{Denied, Limiter, Policy};

    #[test]
    fn no_policy_accepts_everything() {
        let policy = Policy::parse(None).unwrap();
        assert_eq!(policy.admit("anything", None), Ok(()));
        assert!(Limiter::default().allow(policy.rate_per_minute, "anything", "1.2.3.4"));
    }

    #[test]
    fn listed_projects_and_origins_are_enforced() {
        let policy = Policy::parse(Some(
            r#"{"projects":{"shop":{"origins":["https://shop.example"]},"open":{}}}"#,
        ))
        .unwrap();
        assert_eq!(policy.admit("shop", Some("https://shop.example")), Ok(()));
        assert_eq!(policy.admit("shop", Some("https://evil.example")), Err(Denied::Origin));
        assert_eq!(policy.admit("shop", None), Err(Denied::Origin));
        assert_eq!(policy.admit("open", None), Ok(()));
        assert_eq!(policy.admit("other", None), Err(Denied::UnknownProject));
    }

    #[test]
    fn the_limiter_counts_per_project_and_address() {
        let limiter = Limiter::default();
        assert!(limiter.allow(Some(2), "shop", "1.2.3.4"));
        assert!(limiter.allow(Some(2), "shop", "1.2.3.4"));
        assert!(!limiter.allow(Some(2), "shop", "1.2.3.4"));
        assert!(limiter.allow(Some(2), "shop", "5.6.7.8"));
    }

    #[test]
    fn an_unknown_policy_field_is_an_error() {
        assert!(Policy::parse(Some(r#"{"rate":1}"#)).is_err());
    }
}
