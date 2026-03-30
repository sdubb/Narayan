import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ArrowRight, Clock, DollarSign, TrendingUp, Zap } from 'lucide-react';
import '../styles/benefits-scroller.css';

const useCases = [
  {
    id: 1,
    title: 'Brand Monitoring & Protection',
    problem: 'Teams miss fake sites, copied logos, and bad mentions until customers already see them.',
    workflow: 'Narayan checks sites and mentions, spots risky changes, and alerts the team fast.',
    metrics: { timeSaved: '20 hours/week', moneySaved: '$8,500/month', efficiency: '95%' },
    tools: ['Web', 'Social', 'Alerts', 'Audit Log'],
    story: 'Teams catch brand issues early and stop small problems from turning into customer trust problems.',
  },
  {
    id: 2,
    title: 'Revenue Ops Exception Desk',
    problem: 'Deal approvals slow down when pricing, legal, or finance need to weigh in.',
    workflow: 'Narayan gathers the details, sends it to the right people, and records the final decision.',
    metrics: { timeSaved: '18 hours/week', moneySaved: '$9,500/month', efficiency: '96%' },
    tools: ['CRM', 'Billing', 'Slack', 'Approvals'],
    story: 'Teams stop losing deals to slow handoffs and keep approvals easy to track.',
  },
  {
    id: 3,
    title: 'Vendor Onboarding Chase',
    problem: 'Procurement waits on forms, bank details, and signatures before a vendor can start.',
    workflow: 'Narayan asks for missing items, reminds the right person, and tracks what is still open.',
    metrics: { timeSaved: '22 hours/week', moneySaved: '$7,800/month', efficiency: '94%' },
    tools: ['Procurement', 'Email', 'Docs', 'Audit'],
    story: 'Teams cut the back-and-forth that keeps vendors stuck in limbo.',
  },
  {
    id: 4,
    title: 'Support Escalation Bridge',
    problem: 'Urgent tickets bounce between support, product, and engineering because nobody has the full story.',
    workflow: 'Narayan sums up the issue, pulls account history, and sends it to the right owner.',
    metrics: { timeSaved: '26 hours/week', moneySaved: '$11,400/month', efficiency: '93%' },
    tools: ['Zendesk', 'CRM', 'Slack', 'GitHub'],
    story: 'Customer teams keep SLAs on track while engineering gets the right context faster.',
  },
  {
    id: 5,
    title: 'Invoice Reconciliation',
    problem: 'AP teams waste time matching invoices, purchase orders, receipts, and payment status.',
    workflow: 'Narayan matches records, flags mismatches, and sends approved items to accounting.',
    metrics: { timeSaved: '30 hours/week', moneySaved: '$15,500/month', efficiency: '97%' },
    tools: ['OCR', 'Accounting', 'ERP', 'Payments'],
    story: 'Finance teams stop burning hours on manual matching and catch exceptions sooner.',
  },
  {
    id: 6,
    title: 'Security Questionnaire Intake',
    problem: 'Sales cycles stall when security teams need answers for questionnaires and vendor forms.',
    workflow: 'Narayan pulls answers from docs and past reviews, then drafts a full response packet.',
    metrics: { timeSaved: '14 hours/week', moneySaved: '$10,200/month', efficiency: '95%' },
    tools: ['Docs', 'Policies', 'Evidence', 'Forms'],
    story: 'Teams keep deals moving instead of rewriting the same compliance answers.',
  },
  {
    id: 7,
    title: 'Renewal Risk Monitor',
    problem: 'Customer success misses churn signals hiding in usage, support, and contract dates.',
    workflow: 'Narayan watches the signals, highlights risky accounts, and prepares the next follow-up.',
    metrics: { timeSaved: '16 hours/week', moneySaved: '$13,000/month', efficiency: '91%' },
    tools: ['Usage Data', 'Support', 'CRM', 'Alerts'],
    story: 'CS teams act earlier instead of finding out at renewal time that the account was already slipping away.',
  },
  {
    id: 8,
    title: 'Employee Onboarding Chase',
    problem: 'New hires wait on IT access, policy sign-off, equipment, and handoffs before they can start.',
    workflow: 'Narayan tracks each task, reminds the right owner, and checks the onboarding list.',
    metrics: { timeSaved: '20 hours/week', moneySaved: '$8,900/month', efficiency: '95%' },
    tools: ['HRIS', 'IT', 'Policy', 'Task Tracking'],
    story: 'People ops stops losing days to manual coordination across different teams.',
  },
  {
    id: 9,
    title: 'Collections Follow-up Desk',
    problem: 'Accounts receivable teams chase overdue invoices through scattered reminders.',
    workflow: 'Narayan finds aging invoices, sends reminders, and escalates important accounts.',
    metrics: { timeSaved: '24 hours/week', moneySaved: '$14,200/month', efficiency: '92%' },
    tools: ['Accounting', 'Email', 'ERP', 'Escalation Rules'],
    story: 'Finance teams recover cash faster without turning every overdue invoice into a project.',
  },
  {
    id: 10,
    title: 'Policy Acknowledgement Tracker',
    problem: 'Compliance teams need everyone to read and acknowledge new policies.',
    workflow: 'Narayan sends updates, tracks who has confirmed, and reminds the people who have not.',
    metrics: { timeSaved: '12 hours/week', moneySaved: '$5,600/month', efficiency: '98%' },
    tools: ['Policies', 'Email', 'HR', 'Audit Log'],
    story: 'Teams get proof of compliance without chasing people one by one.',
  },
  {
    id: 11,
    title: 'Contract Renewal Coordinator',
    problem: 'Renewals slip because legal, finance, and customer teams act too late.',
    workflow: 'Narayan watches contract dates, pulls the context, and starts the follow-up early.',
    metrics: { timeSaved: '15 hours/week', moneySaved: '$10,800/month', efficiency: '94%' },
    tools: ['Contracts', 'CRM', 'Email', 'Calendar'],
    story: 'Revenue teams keep renewals from turning into a last-minute fire drill.',
  },
];

function formatNumber(text) {
  return String(text || '').replace(/[^0-9.]/g, '');
}

function Metric({ icon: Icon, label, value }) {
  return (
    <div className="benefit-metric">
      <Icon className="benefit-metric-icon" />
      <div>
        <p className="benefit-metric-label">{label}</p>
        <p className="benefit-metric-value">{value}</p>
      </div>
    </div>
  );
}

export default function BenefitsScroller() {
  const [currentIndex, setCurrentIndex] = useState(0);
  const [autoRotate, setAutoRotate] = useState(true);

  useEffect(() => {
    if (!autoRotate) return undefined;
    const timer = window.setInterval(() => {
      setCurrentIndex(prev => (prev + 1) % useCases.length);
    }, 4500);
    return () => window.clearInterval(timer);
  }, [autoRotate]);

  const currentCase = useCases[currentIndex];
  const nextCases = useCases.filter((_, idx) => idx !== currentIndex).slice(0, 5);

  return (
    <section className="benefits-scroller-section">
      <div className="benefits-container">
        <div className="scroller-header">
          <p className="scroller-eyebrow">Narayan in Action</p>
          <h2 className="scroller-title">Workflows teams still do by hand.</h2>
          <p className="scroller-subtitle">
            These are the repeated jobs that create ops drag, missed SLAs, and lost revenue when no workflow owns them.
          </p>
        </div>

        <div className="benefits-grid">
          <motion.article
            key={currentCase.id}
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.35 }}
            className="featured-case"
          >
            <div className="featured-case-top">
              <div>
                <p className="featured-tag">Workflow {currentIndex + 1}</p>
                <h3 className="featured-title">{currentCase.title}</h3>
              </div>
              <button
                type="button"
                onClick={() => setAutoRotate(prev => !prev)}
                className="feature-chip"
              >
                {autoRotate ? 'Auto-rotating' : 'Paused'}
              </button>
            </div>

            <p className="featured-copy">{currentCase.problem}</p>
            <p className="featured-copy" style={{ marginTop: '0.7rem' }}>{currentCase.workflow}</p>

            <div className="featured-tools">
              {currentCase.tools.map(tool => (
                <span key={tool} className="tool-pill">{tool}</span>
              ))}
            </div>

            <div className="featured-metrics">
              <Metric icon={Clock} label="Time saved" value={currentCase.metrics.timeSaved} />
              <Metric icon={DollarSign} label="Cost avoided" value={currentCase.metrics.moneySaved} />
              <Metric icon={TrendingUp} label="Completion rate" value={currentCase.metrics.efficiency} />
            </div>

            <div className="featured-story">
              <p className="story-kicker">Impact</p>
              <p className="story-copy">{currentCase.story}</p>
            </div>
          </motion.article>

          <div className="support-rail">
            <div className="support-panel">
              <p className="panel-label">More workflows that replace manual chase</p>
              <div className="support-list">
                <AnimatePresence mode="popLayout">
                  {nextCases.map(item => (
                    <motion.button
                      key={item.id}
                      type="button"
                      initial={{ opacity: 0, x: 16 }}
                      animate={{ opacity: 1, x: 0 }}
                      exit={{ opacity: 0, x: -16 }}
                      transition={{ duration: 0.25 }}
                      onClick={() => {
                        setCurrentIndex(useCases.findIndex(c => c.id === item.id));
                        setAutoRotate(false);
                      }}
                      className="support-item"
                    >
                      <div className="support-dot" />
                      <div className="min-w-0">
                        <p className="support-title">{item.title}</p>
                        <p className="support-text">{item.problem}</p>
                      </div>
                      <ArrowRight className="support-arrow" />
                    </motion.button>
                  ))}
                </AnimatePresence>
              </div>
            </div>

          </div>
        </div>

        <div className="scroller-footer">
          <button type="button" className="cta-button">
            Start building your agent
            <ArrowRight size={16} />
          </button>
          <div className="cta-note">
            <span className="cta-note-chip" />
            <p>Free for 14 days · No credit card needed.</p>
          </div>
        </div>
      </div>
    </section>
  );
}
