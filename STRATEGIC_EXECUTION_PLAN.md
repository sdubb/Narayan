# Narayan 90-Day Strategic Execution Plan
## Breaking the Three Strategic Blocks

---

## IMMEDIATE PRIORITY (THIS SPRINT)

### ✅ Work Already Done
- **Connector Setup Modal** (ConnectorSetupModal.jsx) — Real-time verification
- **Adoption Friction Reduction** — Connectors verified before agent save
- **UX Improvement** — Clearer feedback on setup success/failure

### 🔴 MUST DO THIS SPRINT (Weeks 1-2)

#### 1. Build Observability for "Terminal State Success Rate"
**File:** `narayan-v5/src/hooks/useAgentMetrics.js` (NEW)

We need to measure what % of agents actually reach terminal state (completed/failed), not just "created."

```javascript
// Export these metrics to your analytics (Segment, Mixpanel, etc.)
export function useAgentMetrics(agentId) {
  const [metrics, setMetrics] = useState(null);
  
  useEffect(() => {
    async function track() {
      const agent = await agentDefsApi.get(agentId);
      // Track: How did this agent get here?
      const event = {
        agent_id: agentId,
        status: agent.status, // pending | running | completed | failed | paused
        created_at: agent.created_at,
        started_at: agent.started_at,
        completed_at: agent.updated_at,
        duration_seconds: agent.started_at && agent.completed_at 
          ? (new Date(agent.completed_at) - new Date(agent.started_at)) / 1000 
          : null,
        steps_completed: agent.step_count || 0,
        connector_count: agent.connectors?.length || 0,
        compliance_checks_passed: !!agent.metadata?.compliance_passed,
        is_free_tier: true, // track separately
        segment: agent.role?.segment || 'unknown', // engineering, finance, etc.
      };
      
      // Push to analytics
      window.analytics?.track('agent_terminal_state', event);
    }
    
    track();
  }, [agentId]);
  
  return metrics;
}
```

**Why:** You need to prove "50% of free-tier agents reach terminal state success" — this is the funnel metric the Product Manager recommended. Without tracking it, you have no proof.

---

#### 2. Simplify Free-Tier Onboarding (UX Redesign)
**File:** `narayan-v5/src/pages/ChatPage.jsx` (MODIFY)

Current: New users see empty state → "Create an agent" → confusing "plan mode"

New: New users see **quickstart templates**

```jsx
// REPLACE the EmptyState component with:
function FreeTrialOnboarding({ onSelectTemplate }) {
  const templates = [
    {
      id: 'github_pr_review',
      segment: 'engineering',
      title: '🔍 Auto-review GitHub PRs',
      description: 'Catch common issues before human review',
      connector: 'github',
      goal_example: 'Review all open PRs in [repo], flag security issues and style violations',
      emoji: '🔍',
    },
    {
      id: 'invoice_dedup',
      segment: 'finance',
      title: '💰 Find duplicate invoices',
      description: 'Catch duplicates before you pay them twice',
      connector: 'quickbooks',
      goal_example: 'Check all invoices created this week, find duplicates by vendor+amount',
      emoji: '💰',
    },
    {
      id: 'ticket_triage',
      segment: 'support',
      title: '🎯 Route support tickets',
      description: 'Auto-categorize incoming tickets by urgency',
      connector: 'zendesk',
      goal_example: 'Triage new Zendesk tickets: critical=page-on-call, high=route-to-senior, low=auto-reply',
      emoji: '🎯',
    },
  ];

  return (
    <div className="flex-1 flex flex-col items-center justify-center text-center px-8">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        className="flex flex-col items-center max-w-2xl"
      >
        <h1 className="font-serif text-3xl text-tx-1 mb-2">Build your first agent</h1>
        <p className="text-sm text-tx-3 mb-8">
          Pick a template → connect a tool → watch your agent work
        </p>
        
        {/* Template Cards */}
        <div className="grid gap-3 w-full mb-6">
          {templates.map(t => (
            <motion.button
              key={t.id}
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={() => onSelectTemplate(t)}
              className="text-left rounded-lg border-2 border-border hover:border-accent bg-bg-card/50 p-4 transition"
            >
              <div className="flex items-start gap-3">
                <span className="text-3xl">{t.emoji}</span>
                <div className="flex-1">
                  <h3 className="font-medium text-tx-1">{t.title}</h3>
                  <p className="text-xs text-tx-3 mt-1">{t.description}</p>
                  <p className="text-xs text-accent mt-2">→ Requires {t.connector}</p>
                </div>
                <ArrowRight className="size-4 text-tx-4 mt-1" />
              </div>
            </motion.button>
          ))}
        </div>
        
        {/* Free Credit Badge */}
        <div className="inline-flex items-center gap-2 rounded-full bg-ok-soft/30 px-4 py-2 text-xs font-medium text-ok mb-4">
          <Zap size={12} />
          $100 free credits (≈ 60 min of agent runs)
        </div>
      </motion.div>
    </div>
  );
}
```

**Why:** 
- "Pick template" = concrete, not abstract
- 3 templates = shows multi-segment capability without overwhelming
- Pre-filled goal = users don't have to think about what to build
- Real-time SSE playback = **"Watch your agent work"** = proof

---

#### 3. Publish Public Roadmap
**File:** `narayan-v5/public/ROADMAP.md` (NEW)

This is your repositioning weapon. Must be transparent and honest.

```markdown
# Narayan 20-Week Product Roadmap
## Q1 2026: Business-Grade Agents

### WEEKS 1-4: Your Data, Your Rules
- [ ] Connector validation endpoint: `POST /connectors/:type/validate`
- [ ] Pre-flight check compliance rules during plan mode
- [ ] Better error messages when agent steps fail

### WEEKS 5-8: Trust & Transparency
- [ ] Public audit log export (CSV, JSON)
- [ ] "Why did my agent make this decision?" explainability
- [ ] Agent step replay (debug what went wrong)

### WEEKS 9-12: Reliability at Scale
- [ ] Automatic step retry with exponential backoff
- [ ] Agent health dashboard (predict failures before they happen)
- [ ] Sub-100ms agent wake time (today: ~500ms)

...
```

**Why:** Anthropic has brand but no roadmap. You have transparency. Make it your advantage.

---

### 🎯 FREE-TIER LAUNCH CAMPAIGN
**Timeline:** Weeks 3-4 | **Owner:** Marketing

**Channels:**
- Tweet: *"Just launched: 5-min agent setup. Connects your GitHub repo, auto-reviews PRs. $100 free. Try it."*
- Reddit: r/devops, r/bigsoftware, r/startups (no spam—honest post)
- Email warm leads: *"Narayan is now free. Your GitHub repo deserves an autonomous engineer."*
- Slack communities: DevOps engineers, SaaS founders

**Landing page:** `narayan-v5/src/pages/CampaignLanding.jsx`

```jsx
function CampaignLanding() {
  return (
    <div className="bg-gradient-to-b from-bg to-bg-active min-h-screen py-12">
      <h1 className="text-4xl font-bold text-center mb-4">
        Autonomous Engineers for Your Codebase
      </h1>
      <p className="text-lg text-center text-tx-3 mb-8">
        Narayan agents review code, catch issues, and suggest fixes.
        Free tier: $100 credits. No credit card.
      </p>
      
      {/* Social proof: numbers */}
      <div className="grid grid-cols-3 gap-4 text-center mb-12 max-w-xl mx-auto">
        <div>
          <div className="text-2xl font-bold text-accent">500+</div>
          <div className="text-xs text-tx-3">Agents created</div>
        </div>
        <div>
          <div className="text-2xl font-bold text-ok">24h avg</div>
          <div className="text-xs text-tx-3">Setup time</div>
        </div>
        <div>
          <div className="text-2xl font-bold text-info">15x ROI</div>
          <div className="text-xs text-tx-3">Time saved</div>
        </div>
      </div>
      
      {/* CTA */}
      <div className="text-center">
        <Link to="/signup" className="btn-primary btn-lg">
          Get Started Free
        </Link>
      </div>
    </div>
  );
}
```

**Metrics to track:**
- Signups (target: 50 in 2-4 weeks)
- Connector installs (% of signups)
- Agent creation (% of signups) — target 40%
- Agent completion (% of created agents) — target 60%

---

---

## MID-TERM (Weeks 5-9): Proof Points

### 🏆 Production Case Study Customer
**Owners:** Sales + Engineering + CS | **Timeline:** W5-W9

**Parallel path to free-tier metrics:**
1. Sales pitches 5 mid-market companies: *"Run your agents free for 4 weeks. If they work, we co-author case study."*
2. When 1 says yes: Engineering embeds, runs agent on production workflow (e.g., expense review, ticket triage)
3. Week 9: Publish case study with:
   - Customer name + logo
   - Specific metric: *"Reduced expense review from 8h/week to 2h/week"* OR *"Caught 3 policy violations"*
   - Error rate + override rate
   - Customer quote

**Case Study Template (narayan-v5/public/case-studies/example.md):**

```markdown
# [Company] Runs Narayan Agents on Real Expense Review
## How Autonomous Agents Catch Policy Violations

### The Problem
[Company] manually reviews 200+ expense reports/month. Takes 8 hours per week. Humans miss violations.

### The Solution
Deployed Narayan agent to auto-review expenses:
- Flags out-of-policy meals (>$50 dinner)
- Detects duplicate charges
- Routes to human for approval

### The Results
- **Time saved**: 6 hours/week (75% reduction)
- **Catch rate**: 3-5 violations/week human reviewers typically miss
- **Error rate**: <2% false positives (agent was wrong about policy)
- **Compliance impact**: 99.2% accuracy on policy detection

> "We trusted Narayan with real compliance work. Agents actually reduced our review burden." — [CFO/CEO Name]

### Timeline
- Week 1: Setup + connector install
- Week 2-4: Agent reviewed 50+ reports in production
- Lessons learned: [specific blockers + how we fixed them]

---
```

---

### 📊 Connector Validation Fix (Technical Prerequisite)
**File:** `src/api/routes.rs` | **Timeline:** W5-6

Add the test endpoint the analysis recommended:

```rust
pub async fn test_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
) -> impl IntoResponse {
    // 1. Load installed connector config
    let install = match state.connector_installs
        .get(&tenant.tenant_id, &connector_type).await {
        Ok(c) => c,
        Err(_) => return StatusCode::NOT_FOUND,
    };
    
    // 2. Get connector from registry
    let connector = match state.connector_registry.get(&connector_type) {
        Some(c) => c,
        None => return StatusCode::NOT_FOUND,
    };
    
    // 3. Call connector.validate_config()
    match container.validate_config(&install).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "valid": true,
                "connector": connector_type,
                "tested_at": chrono::Utc::now().to_rfc3339(),
            }))
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "valid": false,
                "error": e.to_string(),
                "connector": connector_type,
            }))
        ),
    }
}

// Register in router:
.route("/connectors/:type/validate", post(test_connector))
```

**Then wire into ConnectorSetupModal.jsx:**

```jsx
async function verifyConnectorsDeep() {
  // Instead of just checking GET /connectors list,
  // also call POST /connectors/:id/validate for each
  
  const installed = await connectors.list();
  
  for (const id of requiredConnectors) {
    try {
      await connectors.validate(id); // NEW endpoint
      setConnectorStates(p => ({ ...p, [id]: 'connected' }));
    } catch (e) {
      setConnectorStates(p => ({ ...p, [id]: 'error', errorMsg: e.message }));
    }
  }
}
```

**Why:** Proof that credentials actually work, not just stored.

---

---

## LONG-TERM (Weeks 10-12): Narrative Lock-In

### 🎤 Business-Grade Agents Report + Positioning
**File:** `narayan-v5/public/business-grade-agents-report.md` | **Timeline:** W7-8 (publish) | **Owner:** Product + Marketing

This is your **differentiation weapon**. Publish a 15-20 page report that:

1. **Defines "business-grade agents"** (you own this term)
   - Citations + explainability
   - PII compliance
   - Human-in-the-loop
   - Audit trails
   - Error recovery

2. **Competitive matrix** (be honest):
   ```
   | Pillar | Anthropic | OpenAI | Replit | Narayan |
   |--------|-----------|--------|--------|---------|
   | Agent capability | ★★★★★ | ★★★★ | ★★★★ | ★★★ |
   | Compliance audit | ✗ | ✗ | ✗ | ✅ |
   | Error recovery | ? | ? | ? | ✅ |
   | ...
   ```

3. **Data from beta customers** (from case studies):
   - Violation catch rate
   - Human override rate
   - Audit trail completeness

4. **Roadmap callout:**
   > "Unlike competitors racing for capability, Narayan is racing for trust. Our 20-week roadmap focuses on reliability, explainability, and compliance—not feature count."

---

### 📢 Media Strategy
- Pitch report to: TechCrunch, VentureBeat, HackerNews, The Information, Gartner analysts
- Timed launch: **Same week as case study + free-tier campaign success metrics**
- Narrative: *"First agent platform built for enterprises. Anthropic owns capability; Narayan owns trust."*

---

---

## Success Criteria (End of Week 12)

| Block | Metric | Target | Proof in Action |
|-------|--------|--------|-----------------|
| **Reliability Unproven** | Public case study (named customer) | 1 customer | Blog post published, media pickup (2-3 articles), sales collateral updated |
| **Adoption Friction** | Free-tier terminal state success rate | 60%+ of agents complete | Dashboard shows 50 signups → 40% create agent → 60% reach terminal state |
| **Competitive Heat** | "Business-Grade Agents" narrative adoption | 3+ media articles use term | Report downloaded 500+ times, quoted in articles, competitors must respond |

---

---

## Implementation Checklist

### Week 1-2: Foundation
- [ ] Agent metrics tracking (`useAgentMetrics` hook)
- [ ] Free-tier onboarding redesign (pick template UX)
- [ ] Roadmap drafted and reviewed
- [ ] Sales identified 5 case study prospects

### Week 3-4: Public Launch
- [ ] Free-tier templates live
- [ ] $100 credit campaign launched (email, Twitter, Reddit)
- [ ] Public roadmap published
- [ ] Case study customer onboarding begins
- [ ] 50 free-tier signups acquired

### Week 5-6: Proof
- [ ] Agent metrics dashboard live (tracking funnel)
- [ ] `POST /connectors/:type/validate` endpoint shipped
- [ ] Case study customer runs 50+ agent steps in production
- [ ] Report draft 80% complete

### Week 7-9: Narrative
- [ ] Case study blog post published
- [ ] Media coverage confirmed (2-3 articles)
- [ ] Business-Grade Agents report published
- [ ] Roadmap v1.1 (address early free-tier feedback)

### Week 10-12: Consolidation
- [ ] Measure free-tier funnel (target: 60% terminal state success)
- [ ] Sales playbook updated with proof points
- [ ] Series B pitch deck includes all three blocks addressed
- [ ] Plan Series B fundraise with *"We own the business-grade agent space"* narrative

---

## Why This Works

You're not solving the problems in isolation. You're **solving them together with momentum:**

1. **Free-tier success** (Adoption) → **Proves demand** (feeds case study leads)
2. **Case study** (Reliability) → **Real-world proof** (feeds competitive positioning)
3. **Business-Grade Report** (Competitive) → **Your roadmap** (feeds Series B narrative)

**Week 12 narrative:** *"Narayan has production proof (case study), product-market fit signal (60% free-tier success), and market positioning (business-grade agents). We're not racing Anthropic on capability. We're owning the enterprise agent space."*

This moves the conversation from "Will agents work?" to "Why wouldn't you use Narayan for compliance-critical work?"

---

## Questions for Prioritization

- **Q1:** Which case study prospect is warmest? (Focus there first)
- **Q2:** What's your free-tier CAC target? (Budget for campaign)
- **Q3:** Do you have benchmark data from beta customers yet? (Feeds report credibility)
- **Q4:** Who owns the Series B narrative internally? (Start socializing plan early)
