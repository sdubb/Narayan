use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A department or functional sub-group within a parent tenant (company).
/// Teams share the parent tenant's billing plan and quota limits, but can
/// have their own agent namespace, admin users, and policy rules.
/// Identified by a globally-unique `team_id` separate from `tenant_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantTeam {
    /// Globally unique team identifier.
    pub id: String,
    /// Parent company tenant (FK → tenants.id).
    pub tenant_id: String,
    /// Human-readable team name: "Finance", "Clinical Research", "Legal".
    pub name: String,
    /// URL-safe slug, unique within the parent tenant: "finance", "clinical-research".
    pub slug: String,
    pub description: Option<String>,
    pub status: TeamStatus,
    /// Arbitrary metadata (notification hooks, sub-quota overrides, etc.).
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamStatus {
    Active,
    Suspended,
}

/// Maps a tenant account to a role within a team.
/// Since tenants are currently single-user accounts this effectively maps
/// one user to a team role. When multi-user accounts are added later
/// this table already has the right shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub team_id: String,
    /// The member's Narayan tenant account id.
    pub tenant_id: String,
    pub role: TeamMemberRole,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMemberRole {
    /// Can accept/reject boundary handshakes, manage team agents, invite members.
    Admin,
    /// Can create and run agents; cannot manage handshakes or invite members.
    Member,
    /// Read-only access to team agents and the bilateral audit ledger.
    Viewer,
}

impl TeamMemberRole {
    pub fn from_str(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            "viewer" => Self::Viewer,
            _ => Self::Member,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }

    /// Returns true if this role has at least the privilege level of `required`.
    pub fn satisfies(&self, required: &TeamMemberRole) -> bool {
        match required {
            TeamMemberRole::Viewer => true,
            TeamMemberRole::Member => matches!(self, TeamMemberRole::Member | TeamMemberRole::Admin),
            TeamMemberRole::Admin => matches!(self, TeamMemberRole::Admin),
        }
    }
}

impl std::fmt::Display for TeamMemberRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Thin summary returned by list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub status: TeamStatus,
    pub member_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_satisfies_hierarchy() {
        assert!(TeamMemberRole::Admin.satisfies(&TeamMemberRole::Viewer));
        assert!(TeamMemberRole::Admin.satisfies(&TeamMemberRole::Member));
        assert!(TeamMemberRole::Admin.satisfies(&TeamMemberRole::Admin));

        assert!(TeamMemberRole::Member.satisfies(&TeamMemberRole::Viewer));
        assert!(TeamMemberRole::Member.satisfies(&TeamMemberRole::Member));
        assert!(!TeamMemberRole::Member.satisfies(&TeamMemberRole::Admin));

        assert!(TeamMemberRole::Viewer.satisfies(&TeamMemberRole::Viewer));
        assert!(!TeamMemberRole::Viewer.satisfies(&TeamMemberRole::Member));
        assert!(!TeamMemberRole::Viewer.satisfies(&TeamMemberRole::Admin));
    }

    #[test]
    fn role_roundtrip() {
        for s in ["admin", "member", "viewer"] {
            assert_eq!(TeamMemberRole::from_str(s).as_str(), s);
        }
    }
}
