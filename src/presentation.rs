//! Host-owned view identity. Preparing a panel never revokes the source lease.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    #[default]
    Docked,
    Collapsed,
    Floating,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub workspace: String,
    pub session: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub target: Target,
    pub revision: u64,
    pub requested_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub ticket: Ticket,
}
#[derive(Default)]
pub(crate) struct Floating {
    pub active: Option<Ticket>,
    pending: Option<(Ticket, std::time::Instant)>,
    revision: u64,
}
impl Floating {
    pub fn target(&self) -> Option<Ticket> {
        self.pending
            .as_ref()
            .map(|p| p.0.clone())
            .or_else(|| self.active.clone())
    }
    pub fn prepare(&mut self, target: Target) -> Plan {
        if let Some(ticket) = self.target().filter(|t| t.target == target) {
            return Plan { ticket };
        }
        self.revision += 1;
        let ticket = Ticket {
            target,
            revision: self.revision,
            requested_at_ms: timestamp_ms(),
        };
        self.pending = Some((ticket.clone(), std::time::Instant::now()));
        Plan { ticket }
    }
    pub fn validate(&self, target: &Target, revision: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.target()
                .is_some_and(|t| t.revision == revision && &t.target == target),
            "Stale floating view request"
        );
        Ok(())
    }
    pub fn is_pending(&self, revision: u64) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| p.0.revision == revision)
    }
    pub fn commit(&mut self) -> Option<Ticket> {
        if let Some((next, _)) = self.pending.take() {
            return self.active.replace(next);
        }
        None
    }
    pub fn cancel(&mut self, revision: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.0.revision == revision)
        {
            self.pending = None;
        }
    }
    pub fn remove(&mut self, target: &Target) {
        if self.pending.as_ref().is_some_and(|p| &p.0.target == target) {
            self.pending = None;
        }
        if self.active.as_ref().is_some_and(|p| &p.target == target) {
            self.active = None;
        }
    }
    pub fn expire(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.1.elapsed() > std::time::Duration::from_secs(10))
        {
            self.pending = None;
        }
    }
}
pub(crate) fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_millis().min(u64::MAX as u128) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(s: &str) -> Target {
        Target {
            workspace: "project".into(),
            session: s.into(),
        }
    }
    #[test]
    fn preparation_is_idempotent_and_only_latest_ticket_can_commit() {
        let mut f = Floating::default();
        let a = f.prepare(target("a"));
        assert!(f.active.is_none());
        assert_eq!(f.prepare(target("a")).ticket, a.ticket);
        let b = f.prepare(target("b"));
        assert!(f.validate(&a.ticket.target, a.ticket.revision).is_err());
        f.cancel(a.ticket.revision);
        assert_eq!(f.target(), Some(b.ticket.clone()));
        assert!(f.commit().is_none());
        assert_eq!(f.active, Some(b.ticket));
        let c = f.prepare(target("c"));
        f.cancel(c.ticket.revision);
        assert_eq!(f.target(), f.active);
        assert_eq!(f.target().unwrap().target.session, "b");
    }
    #[test]
    fn window_timeout_rolls_back_without_displacing_the_active_view() {
        let mut f = Floating::default();
        let a = f.prepare(target("a"));
        f.commit();
        let b = f.prepare(target("b"));
        f.pending.as_mut().unwrap().1 =
            std::time::Instant::now() - std::time::Duration::from_secs(11);
        f.expire();
        assert_eq!(f.target(), Some(a.ticket));
        assert!(f.validate(&b.ticket.target, b.ticket.revision).is_err());
        let active = f.active.clone().unwrap();
        f.remove(&active.target);
        assert!(f.target().is_none());
    }
}
